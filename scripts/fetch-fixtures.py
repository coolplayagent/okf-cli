"""Explicitly download pinned upstream bundles; normal tests never use the network."""
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import hashlib
import json
import urllib.request

ROOT = Path(__file__).resolve().parents[1] / 'fixtures' / 'upstream'
SOURCES = [
    ('google', 'GoogleCloudPlatform/open-knowledge-format', 'ad30107c31c06aec8a7d5636e0d1058118604e6f', 'bundles/ga4', ['LICENSE.md']),
    ('skills', 'scaccogatto/okf-skills', '49f8fb4987989b7a443ce278c6aa68ba0493b51f', 'examples/sample-bundle', ['LICENSE', 'NOTICE']),
    ('ruby', 'serradura/okf', 'cfb7ffac7fef37d56e97b5b7dd8fe97ffc648c7c', '.okf', ['LICENSE.txt', 'NOTICE']),
]


def fetch(url):
    request = urllib.request.Request(url, headers={'User-Agent': 'okf-cli-fixtures'})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def main():
    manifest = []
    for name, repo, revision, bundle, licenses in SOURCES:
        tree = json.loads(fetch(f'https://api.github.com/repos/{repo}/git/trees/{revision}?recursive=1'))
        assert not tree.get('truncated'), 'incomplete upstream tree'
        paths = [entry['path'] for entry in tree['tree'] if entry['type'] == 'blob'
                 and entry['path'].startswith(bundle + '/') and entry['path'].endswith('.md')]
        assert paths, f'empty bundle: {repo}/{bundle}'
        def download(path):
            data = fetch(f'https://raw.githubusercontent.com/{repo}/{revision}/{path}')
            relative = f'{name}/bundle/{path[len(bundle) + 1:]}' if path.startswith(bundle + '/') else f'{name}/{path}'
            target = ROOT / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            return {'path': relative, 'source_path': path, 'sha256': hashlib.sha256(data).hexdigest()}
        with ThreadPoolExecutor(max_workers=6) as pool:
            files = list(pool.map(download, sorted(paths + licenses)))
        manifest.append({'name': name, 'repository': f'https://github.com/{repo}',
                         'commit': revision, 'bundle': bundle, 'files': files})
        print(name, len(paths), 'Markdown files')
    ROOT.mkdir(parents=True, exist_ok=True)
    (ROOT / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
