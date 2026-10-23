"""Build a reviewable archive; never publish it. Uses only Python's standard library."""
import hashlib
import json
import platform
from pathlib import Path
import subprocess
import sys
import zipfile

root = Path(__file__).resolve().parents[1]
binary = Path(sys.argv[1]).resolve()
version = subprocess.check_output([str(binary), '--version'], text=True).strip()
target = sys.argv[2] if len(sys.argv) > 2 else f'{platform.system().lower()}-{platform.machine().lower()}'
destination = root / 'target' / 'dist'
destination.mkdir(parents=True, exist_ok=True)
archive = destination / f'okf-cli-{version.split()[-1]}-{target}.zip'
files = {binary.name: binary, 'LICENSE': root / 'LICENSE', 'README.md': root / 'README.md',
         'skills/okf-consume/SKILL.md': root / 'skills/okf-consume/SKILL.md',
         'schemas/response.schema.json': root / 'schemas/response.schema.json'}
files.update({f'docs/{p.name}': p for p in (root / 'docs').iterdir() if p.suffix in {'.md', '.json'}})
files.update({p.relative_to(root).as_posix(): p for p in (root / 'examples/knowledge').rglob('*') if p.is_file()})
manifest = {'version': version, 'target': target, 'sha256': {
    name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in files.items()}}
with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED) as output:
    for name, path in files.items():
        output.write(path, name)
    output.writestr('manifest.json', json.dumps(manifest, indent=2) + '\n')
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
archive.with_suffix('.zip.sha256').write_text(f'{digest}  {archive.name}\n', encoding='ascii')
print(archive)
