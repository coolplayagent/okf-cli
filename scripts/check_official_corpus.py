"""Test an unchanged official OKF checkout with a released CLI, in isolation.

Usage: python scripts/check_official_corpus.py BINARY CHECKOUT OUTPUT_DIRECTORY
With --git-url, register and refresh public HTTPS Git bundles instead of local
directories. Full CLI responses are saved as JSONL. No model calls are made.
"""
import argparse
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time


def sha(data):
    return hashlib.sha256(data).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('checkout', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--git-url', help='Public HTTPS repository matching CHECKOUT')
    parser.add_argument('--ref', default='main', help='Remote ref; must resolve to CHECKOUT HEAD')
    parser.add_argument('--bundles', nargs='+', help='Only test these bundle directory names')
    parser.add_argument('--timeout-ms', type=int, default=30000, help='Per-command CLI timeout')
    parser.add_argument('--as-of', default='2026-10-28T00:00:00Z',
                        help='RFC 3339 evaluation time (default: %(default)s)')
    args = parser.parse_args()
    if args.timeout_ms <= 0:
        parser.error('--timeout-ms must be positive')
    try:
        require(datetime.fromisoformat(args.as_of).tzinfo is not None, 'timezone required')
    except ValueError:
        parser.error('--as-of must be an RFC 3339 timestamp with a timezone')
    binary, checkout, output = (p.resolve() for p in (args.binary, args.checkout, args.output))
    output.mkdir(parents=True, exist_ok=False)
    report = {'repository': args.git_url or 'https://github.com/GoogleCloudPlatform/open-knowledge-format',
              'bundles': [], 'checks': [], 'calls': 0, 'as_of': args.as_of,
              'model_evaluation': False, 'source_mode': 'git-https' if args.git_url else 'fs',
              'git_url': args.git_url, 'git_ref': args.ref if args.git_url else None,
              'timeout_ms': args.timeout_ms}
    try:
        git = os.environ.get('OKF_GIT', 'git')
        commit = subprocess.check_output([git, '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip()
        original = {p.relative_to(checkout).as_posix(): sha(p.read_bytes())
                    for p in (checkout / 'bundles').rglob('*') if p.is_file()}
        report.update(commit=commit, binary_sha256=sha(binary.read_bytes()), corpus_sha256=original)
        queries = {'ga4': ('event_name', 'tables/events_.md'),
                   'stackoverflow': ('accepted_answer_id', 'tables/posts_questions.md'),
                   'crypto_bitcoin': ('transaction_hash', 'tables/inputs.md'),
                   'acme_retail': ('revenue', 'metrics/revenue.md')}
        with tempfile.TemporaryDirectory(prefix='okf-official-') as temporary:
            workspace = Path(temporary)
            env = dict(os.environ, OKF_HOME=str(workspace / 'home'))

            def run(*command, allowed=(0,)):
                if command[0] != 'build-info' and '--timeout-ms' not in command:
                    command = (*command, '--timeout-ms', str(args.timeout_ms))
                started = time.perf_counter()
                record = {'args': command}
                report['calls'] += 1
                try:
                    result = subprocess.run([str(binary), '--format', 'json', *command],
                                            cwd=workspace, env=env, capture_output=True,
                                            timeout=max(120, args.timeout_ms / 1000 + 30))
                    record.update(exit_code=result.returncode,
                                  stdout=result.stdout.decode('utf-8', errors='replace'),
                                  stderr=result.stderr.decode('utf-8', errors='replace'))
                    response = json.loads(result.stdout)
                    record['response'] = response
                except Exception as error:
                    record['error'] = str(error)
                    if isinstance(error, subprocess.TimeoutExpired):
                        record.update(stdout=(error.stdout or b'').decode('utf-8', errors='replace'),
                                      stderr=(error.stderr or b'').decode('utf-8', errors='replace'))
                    raise
                finally:
                    record['elapsed_ms'] = round((time.perf_counter() - started) * 1000, 3)
                    with (output / 'trace.jsonl').open('a', encoding='utf-8') as stream:
                        stream.write(json.dumps(record, ensure_ascii=False) + '\n')
                require(result.returncode in allowed, f'{command}: exit {result.returncode}')
                require(not result.stderr, f'{command}: unexpected stderr')
                require(response['schema_version'] == '2.0', 'unexpected response schema')
                if args.git_url:
                    for observation in response.get('bundles', []):
                        require(observation['upstream_revision'] == commit, 'unexpected Git source revision')
                return response

            def check(name, operation):
                try:
                    details = operation()
                    report['checks'].append({'name': name, 'passed': True, 'details': details})
                except Exception as error:
                    report['checks'].append({'name': name, 'passed': False, 'error': str(error)})

            def read_all(name, path, raw, generation=None, paginated=False):
                command = ['read', f'@{name}/{path}']
                if generation:
                    command += ['--indexed', '--generation', generation]
                if paginated:
                    command += ['--budget-bytes', '257']
                chunks, cursor, seen = [], None, set()
                while True:
                    response = run(*command, *(['--cursor', cursor] if cursor else []), allowed=(0, 7))
                    doc = response['data']['document']
                    require(doc['content_sha256'] == sha(raw), f'{path}: content hash mismatch')
                    chunks.append(doc['text'].encode('utf-8'))
                    if not response['page']['has_more']:
                        break
                    cursor = response['page']['next_cursor']
                    require(cursor and cursor not in seen, 'invalid/repeated read cursor')
                    seen.add(cursor)
                require(b''.join(chunks) == raw, f'{path}: original bytes differ')
                return {'pages': len(chunks), 'bytes': len(raw)}

            def search(name, query, expected, documents, mode):
                command = ['search', query] if mode == 'metadata' else (
                    ['search', query, '--full-text'] if mode == 'full_text' else
                    ['context', '--query', query, '--content'])
                response = run(*command, '--bundle', name, '--limit', '100', '--max-scan', '100000',
                               '--budget-bytes', '65536', '--as-of', report['as_of'], allowed=(0, 7))
                items = response['data']['items']
                paths = {item['id'] + '.md' for item in items}
                for item in items:
                    if 'snippet' in item:
                        raw = documents[item['id'] + '.md']
                        snippet = item['snippet']
                        require(raw[snippet['byte_start']:snippet['byte_end']] == snippet['text'].encode('utf-8'),
                                'snippet byte range mismatch')
                        require(item['content_sha256'] == sha(raw), 'snippet source hash mismatch')
                if expected is None:
                    require(not paths, f'absent query returned results: {sorted(paths)}')
                elif mode != 'metadata':
                    require(expected in paths, f'expected evidence missing: {expected}; got {sorted(paths)}')
                return {'query': query, 'paths': sorted(paths), 'expected_found': expected in paths,
                        'diagnostics': response['diagnostics'], 'status': response['status']}

            sources = {p.name: p for p in (checkout / 'bundles').iterdir() if p.is_dir()}
            selected = set(args.bundles) if args.bundles else set(sources)
            require(selected, 'no bundles selected')
            require(not selected - sources.keys(), f'unknown bundle names: {sorted(selected - sources.keys())}')
            require(not selected - queries.keys(), f'unsupported bundles: {sorted(selected - queries.keys())}; select supported bundles with --bundles')
            for name in selected:
                require(any(sources[name].rglob('*.md')), f'{name}: no Markdown documents')
            report['build'] = run('build-info')['data']
            for name in sorted(selected):
                source = sources[name]
                bundle = workspace / name
                shutil.copytree(source, bundle)
                documents = {p.relative_to(bundle).as_posix(): p.read_bytes() for p in bundle.rglob('*.md')}
                row = {'name': name, 'documents': len(documents)}
                report['bundles'].append(row)
                if args.git_url:
                    registered = run('bundle', 'add', name, '--kind', 'git', '--url', args.git_url,
                                     '--ref', args.ref, '--bundle-path', f'bundles/{name}',
                                     '--scope', 'project', '--timeout-ms', '90000')
                    require(registered['data']['bundle']['git']['commit'] == commit, 'remote/local commit mismatch')
                    row['registration'] = registered
                    check(f'{name}:refresh', lambda: require(run(
                        'bundle', 'refresh', name, '--timeout-ms', '90000')['data']['bundle']['git']['commit']
                        == commit, 'remote changed during test'))
                else:
                    run('bundle', 'add', name, '--kind', 'fs', '--path', str(bundle), '--scope', 'project')
                validation = run('validate', name, '--max-scan', '100000', allowed=(0, 1, 7))
                row['validation'] = validation
                check(f'{name}:validation-complete', lambda: require(
                    validation['data']['failures'] == 0 and validation['data']['link_check']['complete']
                    and validation['data']['resource_check']['complete'], 'validation failed or incomplete'))
                if validation['data']['warnings']:
                    check(f'{name}:strict-warning-gate', lambda: run(
                        'validate', name, '--max-scan', '100000', '--max-warnings', '0', allowed=(1,)))
                check(f'{name}:context', lambda: run('context', '--bundle', name, allowed=(0, 7)))
                directories = {Path('.')} | {Path(path).parent for path in documents}
                for directory in sorted(directories):
                    check(f'{name}:browse:{directory.as_posix()}', lambda d=directory: run(
                        'browse', f'@{name}' + (f'/{d.as_posix()}' if d != Path('.') else ''),
                        '--limit', '100', '--max-scan', '100000', allowed=(0, 7)))
                for path, raw in sorted(documents.items()):
                    check(f'{name}:read:{path}', lambda p=path, r=raw: read_all(name, p, r))
                query, expected = queries[name]
                longest = max(documents, key=lambda path: len(documents[path]))
                check(f'{name}:live-pagination', lambda: read_all(name, longest, documents[longest], paginated=True))
                check(f'{name}:metadata-search', lambda: search(name, query, expected, documents, 'metadata'))
                synced = run('sync', name, '--max-scan', '100000', allowed=(0, 7))
                row['sync'] = synced
                generation = synced['data'].get('generation')
                if not synced['data'].get('published') or not generation:
                    report['checks'].append({'name': f'{name}:sync', 'passed': False, 'error': 'index not published'})
                    continue
                check(f'{name}:index-document-count', lambda: require(synced['data']['documents'] == len(documents),
                                                                      'indexed document count differs'))
                check(f'{name}:index-verify', lambda: run('index', 'verify', name, '--max-scan', '100000'))
                saved_env = env.copy()
                if args.git_url:
                    # A closed local proxy makes network fetching unavailable without changing host networking.
                    env.update(HTTPS_PROXY='http://127.0.0.1:1', HTTP_PROXY='http://127.0.0.1:1',
                               https_proxy='http://127.0.0.1:1', http_proxy='http://127.0.0.1:1',
                               NO_PROXY='', no_proxy='', ALL_PROXY='http://127.0.0.1:1')
                    def failed_refresh():
                        failed = run('bundle', 'refresh', name, '--timeout-ms', '10000', allowed=(5,))
                        require(failed['data']['error']['code'] == 'GIT_TRANSPORT_FAILED', 'unexpected fetch failure')
                        return failed
                    check(f'{name}:offline-refresh-fails', failed_refresh)
                    check(f'{name}:cached-git-read-after-failed-refresh', lambda: read_all(name, expected, documents[expected]))
                    # Prove indexed reads do not need Git or its downloaded object store.
                    mirror = workspace / 'home/git'
                    mirror.rename(workspace / 'home/git-unavailable')
                    env['OKF_GIT'] = str(workspace / 'missing-git.exe')
                # Only the isolated copy is renamed; the official checkout is never modified.
                bundle.rename(workspace / f'{name}-offline')
                for path, raw in sorted(documents.items()):
                    check(f'{name}:offline-read:{path}', lambda p=path, r=raw: read_all(name, p, r, generation))
                check(f'{name}:offline-pagination', lambda: read_all(name, longest, documents[longest], generation, True))
                for mode in ('full_text', 'context'):
                    check(f'{name}:offline-{mode}', lambda m=mode: search(name, query, expected, documents, m))
                    absent = 'okf-absent-evidence-7e086bd992'
                    require(all(absent.encode() not in raw for raw in documents.values()), 'negative query exists in corpus')
                    check(f'{name}:offline-{mode}-no-match', lambda m=mode: search(name, absent, None, documents, m))
                if args.git_url:
                    (workspace / 'home/git-unavailable').rename(workspace / 'home/git')
                    env.clear()
                    env.update(saved_env)
                print(f'{name}: {len(documents)} documents tested', flush=True)
            check('source-checkout-unchanged', lambda: require(original == {
                p.relative_to(checkout).as_posix(): sha(p.read_bytes())
                for p in (checkout / 'bundles').rglob('*') if p.is_file()}, 'source corpus changed'))
    except Exception as error:
        report['checks'].append({'name': 'execution', 'passed': False, 'error': str(error)})
    report['summary'] = {'documents': sum(row['documents'] for row in report['bundles']),
                         'checks': len(report['checks']),
                         'failures': [item for item in report['checks'] if not item['passed']]}
    (output / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(report['summary'], ensure_ascii=False, indent=2))
    return bool(report['summary']['failures'])


if __name__ == '__main__':
    raise SystemExit(main())
