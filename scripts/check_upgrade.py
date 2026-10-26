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
        shutil.copy2(source, binary)
        binary.chmod(0o755)

    def run(*args):
        result = subprocess.run([str(binary), '--format', 'json', *args], env=env, cwd=root,
                                capture_output=True, timeout=60, check=True)
        assert not result.stderr
        response = json.loads(result.stdout)
        assert response['status'] == 'ok', response
        return response['data']

    install(before)
    old_info = run('build-info')
    run('bundle', 'add', 'book', '--path', str(bundle))
    generation = run('sync', 'book')['generation']
    binding = run('bundle', 'show', 'book')
    bundle.rename(root / 'offline')
    install(after)
    new_info = run('build-info')
    for source in (after, before):
        install(source)
        assert run('bundle', 'show', 'book') == binding
        document = run('read', '@book/note', '--indexed', '--generation', generation)['document']
        assert document['text'] == raw
        assert run('index', 'status', 'book')['generation'] == generation
    assert (root / 'offline/note.md').read_bytes() == raw.encode('utf-8')
print(json.dumps({'status': 'passed', 'before': old_info, 'after': new_info,
                  'checks': ['install', 'upgrade', 'offline generation read', 'rollback', 'source preserved']}))
