"""Exercise release aggregation with valid inputs and corrupted release sets."""
import hashlib
import json
from pathlib import Path
import tempfile
import tomllib
import unittest
import zipfile

from assemble_release import ROOT, TARGETS, assemble
from verify_skill_package import verify


class ReleaseAssemblyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.source = Path(self.temp.name) / 'inputs'
        self.source.mkdir()
        self.output = Path(self.temp.name) / 'output'
        self.version = tomllib.loads((ROOT / 'Cargo.toml').read_text(encoding='utf-8'))['workspace']['package']['version']
        self.commit = 'a' * 40
        for target in TARGETS:
            self.write_package(target)

    def write_package(self, platform_target, **changes):
        target = platform_target
        binary = 'okf.exe' if 'windows' in target else 'okf'
        files = {binary: target.encode(), 'skills/okf-cli/SKILL.md': (ROOT / 'skills/okf-cli/SKILL.md').read_bytes()}
        manifest = dict(version=self.version, target=target, source_commit=self.commit,
                        source_dirty=False, tag=None,
                        sha256={name: hashlib.sha256(data).hexdigest() for name, data in files.items()})
        manifest.update(changes)
        path = self.source / f'okf-cli-{self.version}-{target}.zip'
        with zipfile.ZipFile(path, 'w') as archive:
            for name, data in files.items():
                archive.writestr(name, data)
            archive.writestr('manifest.json', json.dumps(manifest))
        path.with_suffix('.zip.sha256').write_text(
            f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n', encoding='ascii')

    def test_complete_skill_and_checksums(self):
        skill = assemble(self.source, self.output, self.commit)
        with zipfile.ZipFile(skill) as archive:
            for target, asset in TARGETS.items():
                name = f'okf-cli/assets/{asset}'
                self.assertEqual(archive.read(name), target.encode())
                self.assertTrue(archive.getinfo(name).external_attr >> 16 & 0o111)
            manifest = json.loads(archive.read('okf-cli/manifest.json'))
            for name, digest in manifest['sha256'].items():
                self.assertEqual(hashlib.sha256(archive.read(f'okf-cli/{name}')).hexdigest(), digest)
        for line in (self.output / 'SHA256SUMS').read_text().splitlines():
            digest, name = line.split()
            self.assertEqual(hashlib.sha256((self.output / name).read_bytes()).hexdigest(), digest)

    def test_mixed_or_dirty_sources_and_wrong_metadata(self):
        target = next(iter(TARGETS))
        for changes in ({'source_commit': 'b' * 40}, {'source_dirty': True},
                        {'version': '9.9.9'}, {'target': 'wrong'}, {'tag': 'v9.9.9'},
                        {'sha256': {}}):
            with self.subTest(changes=changes):
                self.write_package(target, **changes)
                with self.assertRaises(ValueError):
                    assemble(self.source, self.output, self.commit)
                self.assertFalse(self.output.exists())

    def test_missing_and_extra_inputs(self):
        extra = self.source / 'unexpected.zip'
        extra.touch()
        with self.assertRaises(ValueError):
            assemble(self.source, self.output, self.commit)
        extra.unlink()
        next(self.source.glob('*.zip')).unlink()
        with self.assertRaises(ValueError):
            assemble(self.source, self.output, self.commit)

    def test_corrupt_archive(self):
        path = next(self.source.glob('*.zip'))
        path.write_bytes(path.read_bytes() + b'corrupt')
        with self.assertRaisesRegex(ValueError, 'checksum'):
            assemble(self.source, self.output, self.commit)

    def test_tagged_release(self):
        tag = f'v{self.version}'
        for target in TARGETS:
            self.write_package(target, tag=tag)
        assemble(self.source, self.output, self.commit, tag)

    def test_skill_verifier_rejects_tampered_binary(self):
        skill = assemble(self.source, self.output, self.commit)
        with zipfile.ZipFile(skill) as archive:
            files = {name: archive.read(name) for name in archive.namelist()}
        files['okf-cli/assets/windows-x86_64/okf.exe'] = b'changed'
        with zipfile.ZipFile(skill, 'w') as archive:
            for name, data in files.items():
                archive.writestr(name, data)
        skill.with_suffix('.zip.sha256').write_text(
            f'{hashlib.sha256(skill.read_bytes()).hexdigest()}  {skill.name}\n', encoding='ascii')
        with self.assertRaisesRegex(ValueError, 'content hash'):
            verify(skill)

    def test_unsafe_package_entry(self):
        path = next(self.source.glob('*.zip'))
        with zipfile.ZipFile(path, 'a') as archive:
            archive.writestr('../escape', b'unsafe')
        path.with_suffix('.zip.sha256').write_text(
            f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n', encoding='ascii')
        with self.assertRaisesRegex(ValueError, 'unsafe archive path'):
            assemble(self.source, self.output, self.commit)


if __name__ == '__main__':
    unittest.main()
