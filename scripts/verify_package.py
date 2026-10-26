"""Verify hashes and run acceptance on an extracted release archive."""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import subprocess
import sys
import tempfile
import zipfile


def require(condition, message):
    if not condition:
        raise SystemExit(message)


root = Path(__file__).resolve().parents[1]
archive = Path(sys.argv[1]).resolve()
expected = archive.with_suffix('.zip.sha256').read_text(encoding='ascii').split()[0]
require(hashlib.sha256(archive.read_bytes()).hexdigest() == expected, 'archive hash mismatch')
with tempfile.TemporaryDirectory(prefix='okf-package-') as temporary:
    destination = Path(temporary)
    with zipfile.ZipFile(archive) as package:
        for name in package.namelist():
            path = PurePosixPath(name)
            require(not path.is_absolute() and '..' not in path.parts and ':' not in name and '\\' not in name, f'unsafe archive path: {name}')
        package.extractall(destination)
    manifest = json.loads((destination / 'manifest.json').read_text(encoding='utf-8'))
    actual = {p.relative_to(destination).as_posix() for p in destination.rglob('*') if p.is_file()}
    require(actual == set(manifest['sha256']) | {'manifest.json'}, 'archive inventory mismatch')
    for name, digest in manifest['sha256'].items():
        require(hashlib.sha256((destination / name).read_bytes()).hexdigest() == digest, f'file hash mismatch: {name}')
    binary = destination / ('okf.exe' if os.name == 'nt' else 'okf')
    binary.chmod(0o755)
    env = os.environ.copy()
    env['PATH'] = str(Path(env.get('SYSTEMROOT', 'C:/Windows')) / 'System32') if os.name == 'nt' else '/nonexistent'
    env['OKF_HOME'] = str(destination / 'home')
    info = json.loads(subprocess.check_output([str(binary), '--format', 'json', 'build-info'], env=env, cwd=destination))['data']
    require(all(info[key] == manifest[key] for key in ('version', 'target', 'source_commit', 'source_dirty')), 'binary/manifest build-info mismatch')
    require(subprocess.check_output([str(binary), '--version'], text=True).strip() == f'okf {manifest["version"]}', 'binary/manifest version mismatch')
    installed = destination / 'installed-skill'
    subprocess.run([str(binary), 'skill', 'install', str(installed)], env=env, cwd=destination,
                   check=True, stdout=subprocess.DEVNULL)
    require((installed / 'SKILL.md').read_bytes() == (destination / 'skills/okf-consume/SKILL.md').read_bytes(), 'binary/archive embedded skill mismatch')
    for script in ('smoke.py', 'acceptance.py'):
        subprocess.run([sys.executable, str(root / 'scripts' / script), str(binary)],
                       env=env, check=True, timeout=300)
print('Extracted package, hashes and standalone acceptance passed')
