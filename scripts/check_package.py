"""Exercise package mismatch rejection without altering the project checkout."""
from pathlib import Path
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
import zipfile

root = Path(__file__).resolve().parents[1]
binary = Path(sys.argv[1]).resolve()


def expect_rejection(script, arguments, message, cwd):
    for options in ([], ['-O']):
        result = subprocess.run([sys.executable, *options, str(script), *map(str, arguments)],
                                cwd=cwd, capture_output=True, timeout=60)
        if result.returncode == 0 or message not in result.stderr.decode(errors='replace'):
            raise AssertionError((options, arguments, result.stdout, result.stderr))


with tempfile.TemporaryDirectory(prefix='okf-package-negative-') as temporary:
    isolated = Path(temporary)
    (isolated / 'scripts').mkdir()
    shutil.copyfile(root / 'scripts/package.py', isolated / 'scripts/package.py')
    shutil.copyfile(root / 'Cargo.toml', isolated / 'Cargo.toml')
    skill = isolated / 'skills/okf-cli/SKILL.md'
    skill.parent.mkdir(parents=True)
    shutil.copyfile(root / 'skills/okf-cli/SKILL.md', skill)
    def rejected(message, *arguments):
        expect_rejection(isolated / 'scripts/package.py', [binary, *arguments], message, isolated)
        if (isolated / 'target/dist').exists():
            raise AssertionError('rejected package created an output directory')
    rejected('requested target does not match', 'wrong-platform-target')
    rejected('tag/workspace version mismatch', '--tag', 'v9999.0.0')
    skill.write_text('stale embedded skill', encoding='utf-8')
    rejected('rebuild binary after changing')
    shutil.copyfile(root / 'skills/okf-cli/SKILL.md', skill)
    (isolated / 'Cargo.toml').write_text('[workspace.package]\nversion = "9999.0.0"\n', encoding='utf-8')
    rejected('binary/workspace version mismatch')
    archive = isolated / 'invalid.zip'
    for message, files, valid_hash in (
        ('archive hash mismatch', {}, False),
        ('unsafe archive path', {'../escape.txt': 'invalid'}, True),
        ('archive inventory mismatch', {'manifest.json': json.dumps({'sha256': {'missing.txt': '0' * 64}})}, True),
        ('file hash mismatch', {'manifest.json': json.dumps({'sha256': {'file.txt': '0' * 64}}), 'file.txt': 'modified'}, True),
    ):
        with zipfile.ZipFile(archive, 'w') as output:
            for name, content in files.items():
                output.writestr(name, content)
        digest = hashlib.sha256(archive.read_bytes()).hexdigest() if valid_hash else '0' * 64
        archive.with_suffix('.zip.sha256').write_text(digest, encoding='ascii')
        expect_rejection(root / 'scripts/verify_package.py', [archive], message, isolated)
print('Package rejects mismatched target, tag, embedded skill and version in normal and optimized Python modes')
print('Verification rejects corrupt hashes, unsafe paths and mismatched inventories in both Python modes')
