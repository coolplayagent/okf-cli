"""Validate the three platform packages and assemble an offline Agent Skill."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import tomllib
import zipfile


TARGETS = {
    'x86_64-unknown-linux-gnu': 'linux-x86_64/okf',
    'x86_64-pc-windows-msvc': 'windows-x86_64/okf.exe',
    'aarch64-apple-darwin': 'macos-arm64/okf',
}
ROOT = Path(__file__).resolve().parents[1]


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def read_package(path):
    checksum = path.with_suffix('.zip.sha256').read_text(encoding='ascii').split()
    require(checksum == [digest(path.read_bytes()), path.name], 'archive checksum mismatch')
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        require(len(names) == len(set(names)), 'duplicate archive entries')
        for name in names:
            parts = PurePosixPath(name)
            require(not parts.is_absolute() and '..' not in parts.parts
                    and ':' not in name and '\\' not in name, 'unsafe archive path')
        files = {name: archive.read(name) for name in names}
    manifest = json.loads(files.pop('manifest.json'))
    require({name: digest(data) for name, data in files.items()} == manifest['sha256'],
            'package inventory or content hash mismatch')
    return manifest, files


def assemble(source, destination, expected_commit, tag=None):
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text(encoding='utf-8'))['workspace']['package']['version']
    require(tag is None or tag == f'v{version}', 'tag/workspace version mismatch')
    expected = {f'okf-cli-{version}-{target}.zip' for target in TARGETS}
    require({p.name for p in source.iterdir()} == expected | {f'{name}.sha256' for name in expected},
            'expected exactly three platform archives and their checksums')
    skill = (ROOT / 'skills/okf-cli/SKILL.md').read_bytes()
    bundled = {'SKILL.md': skill, 'LICENSE': (ROOT / 'LICENSE').read_bytes()}
    packages = []
    for target, asset in TARGETS.items():
        path = source / f'okf-cli-{version}-{target}.zip'
        manifest, files = read_package(path)
        require(manifest['version'] == version and manifest['target'] == target,
                'package version or target mismatch')
        require(manifest['source_commit'] == expected_commit and manifest['source_dirty'] is False,
                'packages must come from the same clean source commit')
        require(manifest['tag'] == tag, 'package tag mismatch')
        require(files['skills/okf-cli/SKILL.md'] == skill, 'package skill mismatch')
        binary = 'okf.exe' if 'windows' in target else 'okf'
        bundled[f'assets/{asset}'] = files[binary]
        packages.append({'name': path.name, 'sha256': digest(path.read_bytes()), 'target': target})
    # Validate every input before writing any output.
    destination.mkdir(parents=True, exist_ok=True)
    require(not any(destination.iterdir()), 'output directory must be empty')
    skill_path = destination / f'okf-cli-skill-{version}.zip'
    skill_manifest = {'version': version, 'source_commit': expected_commit, 'tag': tag,
                      'sha256': {name: digest(data) for name, data in bundled.items()}}
    with zipfile.ZipFile(skill_path, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
        for name, data in bundled.items():
            entry = zipfile.ZipInfo(f'okf-cli/{name}')
            entry.create_system = 3
            entry.external_attr = (0o100755 if name.startswith('assets/') else 0o100644) << 16
            entry.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(entry, data)
        archive.writestr('okf-cli/manifest.json', json.dumps(skill_manifest, indent=2) + '\n')
    for path in source.iterdir():
        (destination / path.name).write_bytes(path.read_bytes())
    skill_path.with_suffix('.zip.sha256').write_text(
        f'{digest(skill_path.read_bytes())}  {skill_path.name}\n', encoding='ascii')
    packages.append({'name': skill_path.name, 'sha256': digest(skill_path.read_bytes()), 'kind': 'skill'})
    (destination / 'release-set.json').write_text(json.dumps({
        'version': version, 'source_commit': expected_commit, 'tag': tag, 'files': packages,
    }, indent=2) + '\n', encoding='utf-8')
    (destination / 'SHA256SUMS').write_text(''.join(
        f'{digest(p.read_bytes())}  {p.name}\n'
        for p in sorted(destination.iterdir()) if p.is_file()), encoding='ascii')
    return skill_path


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--tag')
    args = parser.parse_args()
    print(assemble(args.source, args.destination, args.commit, args.tag))
