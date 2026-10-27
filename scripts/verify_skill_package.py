"""Verify an assembled Skill and run its native binary without build tools on PATH."""
import argparse
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile
import zipfile

from assemble_release import ROOT, TARGETS, digest, require


def verify(path):
    require(path.with_suffix('.zip.sha256').read_text(encoding='ascii').split()
            == [digest(path.read_bytes()), path.name], 'skill archive checksum mismatch')
    with zipfile.ZipFile(path) as archive, tempfile.TemporaryDirectory(prefix='okf-skill-') as temp:
        manifest = json.loads(archive.read('okf-cli/manifest.json'))
        expected = {'SKILL.md', 'LICENSE'} | {f'assets/{asset}' for asset in TARGETS.values()}
        require(set(manifest['sha256']) == expected, 'skill manifest inventory mismatch')
        names = archive.namelist()
        require(len(names) == len(set(names)) and set(names)
                == {f'okf-cli/{name}' for name in expected | {'manifest.json'}},
                'skill archive inventory mismatch')
        for name, checksum in manifest['sha256'].items():
            require(digest(archive.read(f'okf-cli/{name}')) == checksum, 'skill content hash mismatch')
        archive.extractall(temp)
        system, machine = platform.system(), platform.machine().lower()
        target = {('Windows', 'amd64'): 'x86_64-pc-windows-msvc',
                  ('Linux', 'x86_64'): 'x86_64-unknown-linux-gnu',
                  ('Darwin', 'arm64'): 'aarch64-apple-darwin'}.get((system, machine))
        require(target is not None, f'unsupported verification host: {system}/{machine}')
        binary = Path(temp) / 'okf-cli/assets' / TARGETS[target]
        binary.chmod(0o755)
        env = dict(os.environ, OKF_HOME=str(Path(temp) / 'home'))
        env['PATH'] = str(Path(env.get('SYSTEMROOT', 'C:/Windows')) / 'System32') if os.name == 'nt' else '/nonexistent'
        info = json.loads(subprocess.check_output(
            [str(binary), 'build-info', '--format', 'json'], env=env, cwd=temp))['data']
        require(info['target'] == target and info['source_dirty'] is False
                and all(info[key] == manifest[key] for key in ('version', 'source_commit')),
                'skill binary build-info mismatch')
        installed = Path(temp) / 'installed'
        subprocess.run([str(binary), 'skill', 'install', str(installed)], env=env, cwd=temp,
                       check=True, stdout=subprocess.DEVNULL)
        require((installed / 'SKILL.md').read_bytes() == (Path(temp) / 'okf-cli/SKILL.md').read_bytes(),
                'skill binary embedded instructions mismatch')
        subprocess.run([sys.executable, str(ROOT / 'scripts/smoke.py'), str(binary)],
                       env=env, cwd=temp, check=True, timeout=300)
    print('Skill hashes, build information, embedded instructions and native smoke checks passed')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archive', type=Path)
    verify(parser.parse_args().archive.resolve())
