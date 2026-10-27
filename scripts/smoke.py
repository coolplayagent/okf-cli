"""Exercise a built binary with no compiler or Git on PATH (local/index operations)."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

binary = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix='okf-install-') as temporary:
    root = Path(temporary)
    installed = root / binary.name
    shutil.copy2(binary, installed)
    installed.chmod(0o755)
    env = os.environ.copy()
    env['PATH'] = str(Path(env.get('SYSTEMROOT', 'C:/Windows')) / 'System32') if os.name == 'nt' else '/nonexistent'
    env['OKF_HOME'] = str(root / 'home')
    bundle = root / 'knowledge'
    bundle.mkdir()
    raw = '---\ntype: Note\ntitle: Smoke\n---\n认证 executor.resource 🙂\n'
    (bundle / 'note.md').write_bytes(raw.encode('utf-8'))
    def run(*args):
        result = subprocess.run([str(installed), '--format', 'json', *args], cwd=root, env=env, capture_output=True, check=True)
        assert not result.stderr, result.stderr
        response = json.loads(result.stdout)
        assert response['schema_version'] == '2.0'
        assert response['status'] == 'ok', response
        return response
    run('bundle', 'add', 'book', '--path', str(bundle))
    run('validate', 'book')
    assert run('read', '@book/note')['data']['document']['text'] == raw
    run('sync', 'book')
    bundle.rename(root / 'offline')
    assert run('read', '@book/note', '--indexed')['data']['document']['text'] == raw
    assert run('search', '认证', '--bundle', 'book', '--full-text')['page']['returned'] == 1
    run('context', '--query', 'executor.resource', '--bundle', 'book', '--content')
    run('skill', 'install', str(root / 'skill'))
    expected = Path(__file__).resolve().parents[1] / 'skills/okf-cli/SKILL.md'
    assert (root / 'skill/SKILL.md').read_bytes() == expected.read_bytes()
print('Standalone binary smoke passed')
