"""Compare production candidate retrieval with path scans using actual CLI calls."""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import time
from jsonschema import Draft202012Validator


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--scale', type=int, default=100)
    args = parser.parse_args()
    if not 1 <= args.scale <= 10000:
        parser.error('--scale must be 1..10000')
    binary = args.binary.resolve()
    validator = Draft202012Validator(json.loads((Path(__file__).resolve().parents[1] / 'schemas/response.schema.json').read_text(encoding='utf-8')))
    runs = []
    with tempfile.TemporaryDirectory(prefix='okf-index-candidates-') as temporary:
        work = Path(temporary)
        root = work / 'book'
        env = dict(os.environ, OKF_HOME=str(work / 'home'))
        docs = {f'a/{i:05}.md': '---\ntype: Note\ntags: [noise]\n---\nsharedterm ordinary background\n' for i in range(args.scale)}
        docs['z/tail.md'] = '---\ntype: Service\ntitle: Metadatum\ntags: [security]\nstale_after: 2031-01-01T00:00:00Z\nverified: {by: "human:test", at: "2029-01-01T00:00:00Z"}\n---\nsharedterm late-evidence customer_id 7.2.0 认证流程 İSTANBUL\n'
        for path, raw in docs.items():
            file = root / path
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(raw.encode('utf-8'))

        def call(*arguments, code=0):
            started = time.perf_counter()
            process = subprocess.run([str(binary), '--format', 'json', *arguments], cwd=work, env=env, capture_output=True, timeout=120)
            assert process.returncode == code, (arguments, process.returncode, process.stdout, process.stderr)
            assert not process.stderr
            response = json.loads(process.stdout)
            validator.validate(response)
            for item in response['data'].get('items', []):
                if 'snippet' not in item:
                    continue
                snippet = item['snippet']
                raw = docs[snippet['path']].encode()
                assert item['content_sha256'] == hashlib.sha256(raw).hexdigest()
                assert raw[snippet['byte_start']:snippet['byte_end']].decode() == snippet['text']
            return response, (time.perf_counter()-started)*1000

        call('bundle', 'add', 'book', '--path', str(root))
        _, sync_ms = call('sync', 'book', '--max-scan', '100000', '--timeout-ms', '120000')
        database = work / 'home/content.sqlite'
        with closing(sqlite3.connect(database)) as sql:
            postings = sql.execute('SELECT COUNT(*) FROM candidate_postings').fetchone()[0]
        database_bytes = database.stat().st_size
        root.rename(work / 'offline')
        queries = ['late-evidence', 'ustomer', 'customer_id', '7.2.0', '认证流程', 'İSTANBUL', '认证', 'Metadatum', 'z/tail', 'expired', 'human-reviewed', 'absent-evidence',
                   'Metadatum customer_id', '认证 İSTANBUL', 't 认证流程', 'late-evidence absent-evidence']
        for mode in ('search', 'context'):
            for query in queries:
                command = [mode, query, '--full-text'] if mode == 'search' else [mode, '--query', query, '--content']
                command += ['--bundle', 'book', '--max-scan', str(len(docs)), '--as-of', '2032-01-01T00:00:00Z']
                baseline, baseline_ms = call(*command)
                candidate, candidate_ms = call(*command, '--candidate-budget', '1000000')
                assert candidate['data']['items'] == baseline['data']['items'], query
                assert candidate['page']['total'] == baseline['page']['total']
                runs.append({'mode': mode, 'query': query, 'path_scan_ms': baseline_ms, 'candidate_ms': candidate_ms, 'path_scan': baseline['budget'], 'candidates': candidate['budget']})
        for flags in ([], ['--dir', 'z'], ['--type', 'Service'], ['--tag', 'security'], ['--trust', 'human-reviewed'], ['--expiry', 'current', '--as-of', '2030-01-01T00:00:00Z']):
            query = 'late-evidence' if not flags or flags[0] == '--dir' else 'sharedterm'
            result, _ = call('search', query, '--bundle', 'book', '--full-text', '--max-scan', '1', '--candidate-budget', '1000000', *flags)
            assert result['page']['total'] == 1
            assert result['budget']['indexed_documents_scanned'] == 1
        baseline, _ = call('search', 'late-evidence', '--bundle', 'book', '--full-text', '--max-scan', '1', code=7)
        assert baseline['data']['items'] == [] and baseline['page']['total'] is None
        for budget in ('0', '1'):
            result, _ = call('search', 'late-evidence', '--bundle', 'book', '--full-text', '--candidate-budget', budget, code=7)
            assert result['page']['total'] is None
            assert result['budget']['candidate_rows_read'] <= int(budget)
        call('search', 'term', '--candidate-budget', '100', code=2)
        call('search', 'term', '--bundle', 'book', '--full-text', '--candidate-budget', '-1', code=2)
        call('search', 'term', '--bundle', 'book', '--full-text', '--candidate-budget', '1000001', code=2)
        call('context', '--bundle', 'book', '--candidate-budget', '100', code=2)
        (work / 'offline').rename(root)
        call('bundle', 'add', 'other', '--path', str(root))
        call('sync', 'other', '--max-scan', '100000', '--timeout-ms', '120000')
        first, _ = call('search', 'late-evidence', '--bundle', 'book', '--bundle', 'other', '--full-text', '--max-scan', '2', '--candidate-budget', '1000')
        second, _ = call('search', 'late-evidence', '--bundle', 'other', '--bundle', 'book', '--full-text', '--max-scan', '2', '--candidate-budget', '1000')
        assert first['data']['items'] == second['data']['items'] and first['page']['total'] == 2
        assert first['budget']['candidate_rows_read'] <= 1000
        for mutation in ("DELETE FROM candidate_postings WHERE gram='lat'", "UPDATE candidate_documents SET metadata='{}'"):
            with closing(sqlite3.connect(database)) as sql:
                sql.execute(mutation)
                sql.commit()
            result, _ = call('index', 'verify', 'book', '--max-scan', '100000', code=5)
            assert result['data']['valid'] is False
            call('search', 'absent-evidence', '--bundle', 'book', '--full-text', '--candidate-budget', '1000', code=5)
            call('sync', 'book', '--max-scan', '100000', '--timeout-ms', '120000')
            call('index', 'verify', 'book', '--max-scan', '100000')
        with closing(sqlite3.connect(database)) as sql:
            # Simulate a legacy snapshot with no derived candidate manifest.
            sql.execute('DELETE FROM candidate_snapshots')
            sql.commit()
        call('search', 'late-evidence', '--bundle', 'book', '--full-text', '--max-scan', '100000')
        call('search', 'late-evidence', '--bundle', 'book', '--full-text', '--candidate-budget', '1000', code=3)
        call('sync', 'book', '--max-scan', '100000', '--timeout-ms', '120000')
        call('search', 'late-evidence', '--bundle', 'book', '--full-text', '--candidate-budget', '1000')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps({'production': True, 'documents': len(docs), 'sync_ms': sync_ms, 'database_bytes': database_bytes, 'postings': postings, 'runs': runs}, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    print(f'Production candidates passed: {len(runs)} comparisons, budgets, filters, multi-bundle, corruption, repair and legacy reads')


if __name__ == '__main__':
    main()
