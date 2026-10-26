"""Verify hashes and run acceptance on an extracted release archive."""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import subprocess
import sys
import tempfile
import zipfile

root = Path(__file__).resolve().parents[1]
archive = Path(sys.argv[1]).resolve()
expected = archive.with_suffix('.zip.sha256').read_text(encoding='ascii').split()[0]
assert hashlib.sha256(archive.read_bytes()).hexdigest() == expected
with tempfile.TemporaryDirectory(prefix='okf-package-') as temporary:
    destination = Path(temporary)
    with zipfile.ZipFile(archive) as package:
        for name in package.namelist():
            path = PurePosixPath(name)
            assert not path.is_absolute() and '..' not in path.parts and ':' not in name and '\\' not in name
        package.extractall(destination)
    manifest = json.loads((destination / 'manifest.json').read_text(encoding='utf-8'))
    actual = {p.relative_to(destination).as_posix() for p in destination.rglob('*') if p.is_file()}
    assert actual == set(manifest['sha256']) | {'manifest.json'}
    for name, digest in manifest['sha256'].items():
        assert hashlib.sha256((destination / name).read_bytes()).hexdigest() == digest, name
    binary = destination / ('okf.exe' if os.name == 'nt' else 'okf')
    binary.chmod(0o755)
    env = os.environ.copy()
    env['PATH'] = str(Path(env.get('SYSTEMROOT', 'C:/Windows')) / 'System32') if os.name == 'nt' else '/nonexistent'
    for script in ('smoke.py', 'acceptance.py'):
        subprocess.run([sys.executable, str(root / 'scripts' / script), str(binary)],
                       env=env, check=True, timeout=300)
print('Extracted package, hashes and standalone acceptance passed')
