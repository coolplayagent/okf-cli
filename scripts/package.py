"""Build a verified release archive locally; never publish it. Python 3.11+."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import tomllib
import zipfile


def require(condition, message):
    if not condition:
        raise SystemExit(message)


root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('target', nargs='?', help='Expected Rust target; must match the binary')
parser.add_argument('--tag', help='Release tag; also requires a clean matching source commit')
args = parser.parse_args()
binary = args.binary.resolve()
version = tomllib.loads((root / 'Cargo.toml').read_text(encoding='utf-8'))['workspace']['package']['version']
require(subprocess.check_output([str(binary), '--version'], text=True).strip() == f'okf {version}', 'binary/workspace version mismatch')
with tempfile.TemporaryDirectory(prefix='okf-package-check-') as temporary:
    env = dict(os.environ, OKF_HOME=str(Path(temporary) / 'home'))
    info = json.loads(subprocess.check_output([str(binary), '--format', 'json', 'build-info'], env=env, cwd=temporary))['data']
    require(info['version'] == version, 'build-info/workspace version mismatch')
    target = info['target']
    require(re.fullmatch(r'[a-zA-Z0-9_]+(?:-[a-zA-Z0-9_]+)+', target), f'invalid binary target: {target}')
    if args.target:
        require(args.target == target, 'requested target does not match the binary')
    installed = Path(temporary) / 'skill'
    subprocess.run([str(binary), 'skill', 'install', str(installed)], env=env, cwd=temporary,
                   check=True, stdout=subprocess.DEVNULL)
    require((installed / 'SKILL.md').read_bytes() == (root / 'skills/okf-consume/SKILL.md').read_bytes(), 'rebuild binary after changing the embedded skill')

if args.tag:
    require(args.tag == f'v{version}', 'tag/workspace version mismatch')
    require(info['source_dirty'] is False, 'release binary must come from a clean checkout')
    git = os.environ.get('OKF_GIT', 'git')
    def git_output(*arguments):
        return subprocess.check_output([git, '-C', str(root), *arguments], text=True).strip()
    require(git_output('status', '--porcelain', '--untracked-files=normal') == '', 'release checkout is dirty')
    require(info['source_commit'] == git_output('rev-parse', 'HEAD') == git_output('rev-parse', f'refs/tags/{args.tag}^{{commit}}'), 'tag/binary/source commit mismatch')

destination = root / 'target/dist'
destination.mkdir(parents=True, exist_ok=True)
archive = destination / f'okf-cli-{version}-{target}.zip'
files = {('okf.exe' if 'windows' in target.split('-') else 'okf'): binary,
         'LICENSE': root / 'LICENSE', 'README.md': root / 'README.md', 'CHANGELOG.md': root / 'CHANGELOG.md',
         'skills/okf-consume/SKILL.md': root / 'skills/okf-consume/SKILL.md',
         'schemas/response.schema.json': root / 'schemas/response.schema.json'}
files.update({f'docs/{p.name}': p for p in sorted((root / 'docs').iterdir()) if p.suffix in {'.md', '.json'}})
files.update({p.relative_to(root).as_posix(): p for p in sorted((root / 'examples/knowledge').rglob('*')) if p.is_file()})
manifest = {**info, 'tag': args.tag, 'sha256': {
    name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in files.items()}}
with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED) as output:
    for name, path in files.items():
        output.write(path, name)
    output.writestr('manifest.json', json.dumps(manifest, indent=2) + '\n')
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
archive.with_suffix('.zip.sha256').write_text(f'{digest}  {archive.name}\n', encoding='ascii')
print(archive)
