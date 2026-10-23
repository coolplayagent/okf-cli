"""Exercise a built CLI against pinned upstream bundles and isolated failure cases.

Usage: python scripts/acceptance.py target/release/okf[.exe]
Uses only the standard library; never modifies the upstream fixtures.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[1]
binary = Path(sys.argv[1]).resolve()
manifest = json.loads((root / 'fixtures/upstream/manifest.json').read_text(encoding='utf-8'))
calls = 0
documents = 0
with tempfile.TemporaryDirectory(prefix='okf-acceptance-') as temporary:
    workspace = Path(temporary)
    env = os.environ.copy()
    env['OKF_HOME'] = str(workspace / 'home')

    def run(*args, code=0):
        global calls
        result = subprocess.run([str(binary), '--format', 'json', *args], cwd=workspace,
                                env=env, capture_output=True, timeout=60)
        calls += 1
        assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
        assert not result.stderr, (args, result.stderr)
        response = json.loads(result.stdout)
        assert response['schema_version'] == '2.0'
        expected_statuses = ('ok',) if code == 0 else ('partial', 'error')
        assert response['status'] in expected_statuses, response
        return response

    for source in manifest:
        name = source['name']
        bundle = workspace / name
        shutil.copytree(root / 'fixtures/upstream' / name / 'bundle', bundle)
        run('bundle', 'add', name, '--path', str(bundle))
        run('validate', name, '--max-scan', '10000')
        synced = run('sync', name, '--max-scan', '10000')
        generation = synced['data']['generation']
        files = [file for file in source['files'] if '/bundle/' in file['path']]
        assert synced['data']['documents'] == len(files)
        for file in files:
            path = file['path'].split('/bundle/', 1)[1]
            expected = (bundle / path).read_bytes()
            assert hashlib.sha256(expected).hexdigest() == file['sha256']
            document = run('read', f'@{name}/{path}')['data']['document']
            assert document['text'].encode('utf-8') == expected
            assert document['content_sha256'] == file['sha256']
        bundle.rename(workspace / (name + '-offline'))
        for file in files:
            path = file['path'].split('/bundle/', 1)[1]
            document = run('read', f'@{name}/{path}', '--indexed', '--generation', generation)['data']['document']
            assert hashlib.sha256(document['text'].encode('utf-8')).hexdigest() == file['sha256']
            assert document['content_sha256'] == file['sha256']
            documents += 1
        print(name, len(files), 'live and offline documents verified', flush=True)

    bundle = workspace / 'edge'
    bundle.mkdir()
    raw = '\ufeff---\r\ntype: Note\r\ntitle: Unicode\r\n---\r\n# 标题\r\n认证 executor.resource İ 🙂\r\n'
    (bundle / '中文.md').write_bytes(raw.encode('utf-8'))
    run('bundle', 'add', 'edge', '--path', str(bundle))
    generation = run('sync', 'edge')['data']['generation']
    for indexed in (False, True):
        args = ('--indexed',) if indexed else ()
        chunks, cursor = [], None
        while True:
            continuation = ('--cursor', cursor) if cursor else ()
            page = run('read', '@edge/中文', '--budget-bytes', '7', *args, *continuation)
            chunks.append(page['data']['document']['text'])
            cursor = page['page']['next_cursor']
            if not page['page']['has_more']:
                break
            assert cursor
        assert ''.join(chunks) == raw
    for command in [('search', '认证', '--full-text'), ('context', '--query', 'executor.resource', '--content')]:
        response = run(*command, '--bundle', 'edge')
        assert response['page']['returned'] == 1
        snippet = response['data']['items'][0]['snippet']
        assert raw.encode('utf-8')[snippet['byte_start']:snippet['byte_end']] == snippet['text'].encode('utf-8')
    (bundle / '中文.md').write_bytes((raw + 'updated\r\n').encode('utf-8'))
    failed = run('sync', 'edge', '--max-scan', '1', code=7)
    assert not failed['data']['published']
    assert run('index', 'status', 'edge')['data']['generation'] == generation
    assert run('read', '@edge/中文', '--indexed', '--generation', generation)['data']['document']['text'] == raw
    assert run('sync', 'edge')['data']['generation'] != generation
    stale = run('read', '@edge/中文', '--indexed', '--generation', generation, code=6)
    assert stale['data']['error']['code'] == 'REVISION_CHANGED'
    cache = workspace / 'home/index.sqlite'
    cache.write_bytes(b'intentionally invalid SQLite database')
    degraded = run('read', '@edge/中文', code=7)
    assert degraded['data']['document']['text'] == raw + 'updated\r\n'
    assert any(d['code'].startswith('CACHE_') for d in degraded['diagnostics'])
    run('read', '@edge/中文', '--indexed')
    archive = Path(run('cache', 'reset')['data']['archive'])
    assert (archive / 'index.sqlite').read_bytes() == b'intentionally invalid SQLite database'
    run('read', '@edge/中文')
    run('index', 'clear', 'edge')
    missing = run('read', '@edge/中文', '--indexed', code=3)
    assert missing['data']['error']['code'] == 'INDEX_MISSING'
    run('bundle', 'remove', 'edge')
    assert (bundle / '中文.md').read_bytes() == (raw + 'updated\r\n').encode('utf-8')
print(json.dumps({'status': 'passed', 'upstream_documents': documents, 'cli_calls': calls}))
