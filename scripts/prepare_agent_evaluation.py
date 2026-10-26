"""Prepare isolated CLI projects and blind task packets; does not call a model."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    output = args.output.resolve()
    if output.exists():
        parser.error('output must not exist; use a new directory for each preparation')
    fixtures = Path(__file__).resolve().parents[1] / 'fixtures/evaluation'
    protocol = json.loads((fixtures / 'agent-tasks.json').read_text(encoding='utf-8'))
    single = json.loads((fixtures / 'tasks.json').read_text(encoding='utf-8'))
    multi = json.loads((fixtures / 'multi-bundle.json').read_text(encoding='utf-8'))
    output.mkdir(parents=True)
    environments = {}
    inventories = {}
    for corpus, bundles in {'single': {'book': {'sync': True, 'documents': single['documents']}}, 'multi': multi['bundles']}.items():
        project = output / corpus
        project.mkdir()
        # Stop registry discovery here, even when output lives inside a Git repo.
        config = project / '.okf-cli'
        config.mkdir()
        (config / 'config.toml').write_text('bundles = []\n', encoding='utf-8')
        home = project / 'home'
        env = dict(os.environ, OKF_HOME=str(home))
        inventories[corpus] = {}
        for name, bundle in bundles.items():
            root = project / 'sources' / name
            for path, raw in bundle['documents'].items():
                file = root / path
                file.parent.mkdir(parents=True, exist_ok=True)
                content = raw.encode('utf-8')
                file.write_bytes(content)
                inventories[corpus][f'@{name}/{path.removesuffix(".md")}'] = hashlib.sha256(content).hexdigest()
            commands = [['bundle', 'add', name, '--path', str(root)]]
            if bundle['sync']:
                commands.append(['sync', name, '--max-scan', '1000'])
            for command in commands:
                subprocess.run([str(binary), '--format', 'json', *command], cwd=project, env=env, check=True, stdout=subprocess.DEVNULL)
        environments[corpus] = {'cwd': str(project), 'OKF_HOME': str(home), 'bundles': list(bundles)}
    tasks = [{k: task[k] for k in ('id', 'corpus', 'prompt', 'arms') if k in task} for task in protocol['tasks']]
    packet = {k: v for k, v in protocol.items() if k != 'tasks'} | {'tasks': tasks, 'binary': str(binary), 'environments': environments}
    (output / 'tasks.json').write_text(json.dumps(packet, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    (output / 'grading.json').write_text(json.dumps({'tasks': protocol['tasks'], 'source_hashes': inventories}, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    manifest = {'protocol': protocol['protocol'], 'model_run': False, 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'fixtures_sha256': {name: hashlib.sha256((fixtures / name).read_bytes()).hexdigest() for name in ('tasks.json', 'multi-bundle.json', 'agent-tasks.json')}, 'build': json.loads(subprocess.check_output([str(binary), '--format', 'json', 'build-info']))['data']}
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n', encoding='utf-8')
    print(f'Prepared {len(tasks)} blind tasks and two CLI projects; no model calls')


if __name__ == '__main__':
    main()
