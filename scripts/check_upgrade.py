"""Verify installing, replacing and rolling back two compatible local CLI binaries.

Usage: python scripts/check_upgrade.py BEFORE AFTER
This checks the supplied pair only; it is not a database migration guarantee.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

before, after = [Path(p).resolve() for p in sys.argv[1:]]
with tempfile.TemporaryDirectory(prefix='okf-upgrade-') as temporary:
    root = Path(temporary)
    binary = root / ('okf.exe' if os.name == 'nt' else 'okf')
    env = dict(os.environ, OKF_HOME=str(root / 'home'))
    env['PATH'] = str(Path(env.get('SYSTEMROOT', 'C:/Windows')) / 'System32') if os.name == 'nt' else '/nonexistent'
    bundle = root / 'book'
    bundle.mkdir()
    raw = '---\ntype: Note\n---\nupgrade evidence 认证\n'
    (bundle / 'note.md').write_bytes(raw.encode('utf-8'))

    def install(source):
        # Windows scanners can briefly retain the executable after the child exits.
        deadline = time.monotonic() + 3
        while True:
            try:
                shutil.copy2(source, binary)
                break
            except PermissionError as error:
                if getattr(error, 'winerror', None) not in (5, 32) or time.monotonic() >= deadline:
                    raise
                time.sleep(0.1)
        binary.chmod(0o755)

    def run(*args, code=0):
        result = subprocess.run([str(binary), '--format', 'json', *args], env=env, cwd=root,
                                capture_output=True, timeout=60)
        assert result.returncode == code, (args, result.returncode, result.stdout, result.stderr)
        assert not result.stderr
        response = json.loads(result.stdout)
        if code == 0:
            assert response['status'] == 'ok', response
        return response['data']

    install(before)
    old_info = run('build-info')
    old_candidates = '--candidate-budget' in subprocess.check_output([str(binary), 'search', '--help'], text=True)
    run('bundle', 'add', 'book', '--path', str(bundle))
    generation = run('sync', 'book')['generation']
    binding = run('bundle', 'show', 'book')
    bundle.rename(root / 'offline')
    install(after)
    new_info = run('build-info')
    new_candidates = '--candidate-budget' in subprocess.check_output([str(binary), 'search', '--help'], text=True)
    for source in (after, before):
        install(source)
        assert run('bundle', 'show', 'book') == binding
        document = run('read', '@book/note', '--indexed', '--generation', generation)['document']
        assert document['text'] == raw
        assert run('index', 'status', 'book')['generation'] == generation
    checks = ['install', 'upgrade', 'offline generation read', 'rollback', 'source preserved']
    if new_candidates:
        install(after)
        (root / 'offline').rename(bundle)
        assert run('sync', 'book')['generation'] == generation
        arguments = ('search', 'evidence', '--bundle', 'book', '--full-text', '--candidate-budget', '1000')
        assert len(run(*arguments)['items']) == 1
        install(before)
        assert run('read', '@book/note', '--indexed')['document']['text'] == raw
        run('sync', 'book')
        install(after)
        if not old_candidates:
            assert run(*arguments, code=5)['error']['code'] == 'INDEX_CANDIDATES_CORRUPT'
            checks.append('legacy writer invalidates candidates')
        run('sync', 'book')
        assert len(run(*arguments)['items']) == 1
        bundle.rename(root / 'offline')
        checks += ['candidate publication', 'rollback reads extended schema', 'candidate repair']
    assert (root / 'offline/note.md').read_bytes() == raw.encode('utf-8')
print(json.dumps({'status': 'passed', 'before': old_info, 'after': new_info,
                  'checks': checks}))
