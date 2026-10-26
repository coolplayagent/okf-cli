"""Measure the per-bundle CLI baseline for future federated retrieval.

No global ranking or global budget is claimed. Each bundle has its own request.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
binary = args.binary.resolve()
dataset = json.loads((root / 'fixtures/evaluation/multi-bundle.json').read_text(encoding='utf-8'))
runs = []
with tempfile.TemporaryDirectory(prefix='okf-multi-') as temporary:
    workspace = Path(temporary)
    env = dict(os.environ, OKF_HOME=str(workspace / 'home'))
    def run(*arguments, code=0):
        result = subprocess.run([str(binary), '--format', 'json', *arguments], env=env, cwd=workspace,
                                capture_output=True, timeout=60)
        assert result.returncode == code, (arguments, result.stdout, result.stderr)
        assert not result.stderr
        return json.loads(result.stdout)
    generations = {}
    for name, bundle in dataset['bundles'].items():
        directory = workspace / name
        directory.mkdir()
        for path, raw in bundle['documents'].items():
            destination = directory / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(raw.encode('utf-8'))
        run('bundle', 'add', name, '--path', str(directory))
        if bundle['sync']:
            run('sync', name, '--max-scan', '1000')
            generations[name] = run('index', 'status', name)['data']['generation']
        directory.rename(workspace / (name + '-offline'))
    for task in dataset['tasks']:
        for mode in ('full_text', 'context'):
            evidence = {}
            missing = []
            disclosed = 0
            for name, bundle in dataset['bundles'].items():
                command = ['search', task['query'], '--full-text'] if mode == 'full_text' else ['context', '--query', task['query'], '--content']
                response = run(*command, '--bundle', name, '--as-of', dataset['as_of'],
                               '--limit', '5', '--budget-bytes', '16384', '--max-scan', '1000',
                               *task['filters'], code=0 if bundle['sync'] else 3)
                if not bundle['sync']:
                    assert response['data']['error']['code'] == 'INDEX_MISSING'
                    missing.append(name)
                    continue
                assert response['data']['generation'] == generations[name]
                disclosed += response['budget']['content_bytes_used']
                for item in response['data']['items']:
                    raw = bundle['documents'][item['id'] + '.md'].encode('utf-8')
                    snippet = item['snippet']
                    assert snippet['text'].encode('utf-8') == raw[snippet['byte_start']:snippet['byte_end']]
                    assert item['content_sha256'] == hashlib.sha256(raw).hexdigest()
                    assert item['generation'] == generations[name]
                    evidence[item['ref']] = snippet['text']
            assert set(evidence) == set(task['expected']), (task['id'], evidence)
            assert all(text in evidence[reference] for reference, text in task['expected'].items())
            runs.append({'task': task['id'], 'mode': mode, 'calls': len(dataset['bundles']),
                         'references': sorted(evidence), 'missing_indexes': missing,
                         'coverage_complete': not missing, 'disclosed_bytes': disclosed,
                         'expected_evidence_verified': True})
report = {'mode': 'per_bundle_baseline', 'as_of': dataset['as_of'], 'tasks': len(dataset['tasks']),
          'bundles': len(dataset['bundles']), 'generations': generations,
          'budget_scope': 'per_request_per_bundle', 'runs': runs}
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
print(f'Cross-bundle baseline passed: {len(runs)} task/mode cases; missing indexes remain explicit')
