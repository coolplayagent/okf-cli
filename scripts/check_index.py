"""Exercise read-only index verification and explicit sync repair through the CLI."""
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
calls = 0
with tempfile.TemporaryDirectory(prefix='okf-index-verify-') as temporary:
    work = Path(temporary)
    home = work / 'home'
    env = dict(os.environ, OKF_HOME=str(home))
    bundle = work / 'book'
    bundle.mkdir()
    for name in ('a.md', 'z.md'):
        (bundle / name).write_text('---\ntype: Note\n---\n认证 evidence\n', encoding='utf-8')

    def run(*args, code=0):
        global calls
        result = subprocess.run([str(binary), '--format', 'json', *args], cwd=work,
                                env=env, capture_output=True, timeout=60)
        calls += 1
        assert result.returncode == code, (args, result.stdout, result.stderr)
        assert not result.stderr
        return json.loads(result.stdout)

    run('bundle', 'add', 'book', '--path', str(bundle))
    run('index', 'verify', 'book', code=3)
    database = home / 'content.sqlite'
    assert not database.exists()
    generation = run('sync', 'book')['data']['generation']

    def dump():
        with closing(sqlite3.connect(database)) as connection:
            return list(connection.iterdump())

    before = dump()
    partial = run('index', 'verify', 'book', '--max-scan', '1', code=7)
    assert partial['data']['valid'] is None and not partial['data']['complete']
    assert partial['data']['documents_checked'] == 1
    assert run('index', 'verify', 'book')['data']['valid'] is True
    assert dump() == before
    for mutation, diagnostic in (
        ("UPDATE documents SET raw='damaged' WHERE path='z.md'", 'INDEX_CORRUPT'),
        ("UPDATE documents SET hash='damaged' WHERE path='z.md'", 'INDEX_CORRUPT'),
        ("UPDATE documents SET generation='damaged' WHERE path='z.md'", 'INDEX_GENERATION_MISMATCH'),
        ("UPDATE documents SET path='../z.md' WHERE path='z.md'", 'INDEX_PATH_INVALID'),
        ("INSERT INTO documents SELECT bundle,'extra.md',hash,raw,generation FROM documents WHERE path='z.md'", 'INDEX_SNAPSHOT_MISMATCH'),
        ("DELETE FROM documents WHERE path='z.md'", 'INDEX_SNAPSHOT_MISMATCH'),
    ):
        with closing(sqlite3.connect(database)) as connection:
            connection.execute(mutation)
            connection.commit()
        damaged = dump()
        result = run('index', 'verify', 'book', code=5)
        assert result['status'] == 'error' and result['data']['valid'] is False
        assert result['data']['complete']
        assert any(d['code'] == diagnostic for d in result['diagnostics'])
        assert dump() == damaged
        assert run('sync', 'book')['data']['generation'] == generation
        assert run('index', 'verify', 'book')['data']['valid'] is True
    bundle.rename(work / 'offline')
    assert run('index', 'verify', 'book')['data']['valid'] is True
    # Exercise the cumulative byte ceiling through the actual CLI. Each body
    # fits the document limit, but the entire inventory exceeds 64 MiB.
    raw = 'x' * (2 * 1024 * 1024)
    digest = hashlib.sha256(raw.encode()).hexdigest()
    with closing(sqlite3.connect(database)) as connection:
        key = connection.execute('SELECT bundle FROM snapshots').fetchone()[0]
        connection.execute('DELETE FROM documents')
        connection.executemany('INSERT INTO documents VALUES (?,?,?,?,?)',
                               ((key, f'{i:03}.md', digest, raw, generation) for i in range(33)))
        connection.commit()
    partial = run('index', 'verify', 'book', code=7)
    assert partial['data']['valid'] is None and not partial['data']['complete']
    assert partial['data']['documents_checked'] == 32
    assert partial['budget']['index_bytes_checked'] == 64 * 1024 * 1024
    assert any(d['code'] == 'INDEX_LIMIT' for d in partial['diagnostics'])
    with closing(sqlite3.connect(database)) as connection:
        connection.execute("UPDATE documents SET hash='damaged' WHERE path='000.md'")
        connection.commit()
    partial = run('index', 'verify', 'book', '--max-scan', '1', code=7)
    assert partial['status'] == 'partial' and partial['data']['valid'] is False
    assert not partial['data']['complete'] and partial['data']['failures'] == 1
    assert {d['code'] for d in partial['diagnostics']} == {'INDEX_CORRUPT', 'SCAN_LIMIT'}
    with closing(sqlite3.connect(database)) as connection:
        connection.execute('PRAGMA user_version=2')
    incompatible = run('index', 'verify', 'book', code=2)
    assert incompatible['status'] == 'error'
    assert incompatible['data']['error']['code'] == 'INDEX_VERSION'
    with closing(sqlite3.connect(database)) as connection:
        assert connection.execute('PRAGMA user_version').fetchone()[0] == 2
print(f'Index verification passed: {calls} CLI calls, partial coverage, corruption, repair and offline reads')
