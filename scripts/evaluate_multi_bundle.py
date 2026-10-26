"""Compare per-bundle calls with one bounded multi-bundle CLI request."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
from measure import run_measured

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
    measurements = []
    def run(*arguments, code=0):
        result, elapsed, peak = run_measured([str(binary), '--format', 'json', *arguments], env=env, cwd=workspace)
        assert result.returncode == code, (arguments, result.stdout, result.stderr)
        assert not result.stderr
        measurements.append((elapsed, peak))
        return json.loads(result.stdout)
    generations = {}

    def verified_evidence(items):
        evidence = {}
        for item in items:
            name, path = item['ref'][1:].split('/', 1)
            assert path == item['id']
            raw = dataset['bundles'][name]['documents'][path + '.md'].encode('utf-8')
            snippet = item['snippet']
            assert snippet['text'].encode('utf-8') == raw[snippet['byte_start']:snippet['byte_end']]
            assert item['content_sha256'] == hashlib.sha256(raw).hexdigest()
            assert item['generation'] == generations[name]
            evidence[item['ref']] = snippet['text']
        return evidence

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
            command = ['search', task['query'], '--full-text'] if mode == 'full_text' else ['context', '--query', task['query'], '--content']
            measurements.clear()
            evidence = {}
            missing = []
            disclosed = 0
            for name, bundle in dataset['bundles'].items():
                response = run(*command, '--bundle', name, '--as-of', dataset['as_of'],
                               '--limit', '5', '--budget-bytes', '16384', '--max-scan', '1000',
                               *task['filters'], code=0 if bundle['sync'] else 3)
                if not bundle['sync']:
                    assert response['data']['error']['code'] == 'INDEX_MISSING'
                    missing.append(name)
                    continue
                assert response['data']['generation'] == generations[name]
                disclosed += response['budget']['content_bytes_used']
                assert all(item['ref'].startswith(f'@{name}/') for item in response['data']['items'])
                evidence.update(verified_evidence(response['data']['items']))
            assert set(evidence) == set(task['expected']), (task['id'], evidence)
            assert all(text in evidence[reference] for reference, text in task['expected'].items())
            runs.append({'task': task['id'], 'mode': mode, 'strategy': 'per_bundle', 'calls': len(dataset['bundles']),
                         'references': sorted(evidence), 'missing_indexes': missing,
                         'coverage_complete': not missing, 'disclosed_bytes': disclosed,
                         'elapsed_ms': sum(m[0] for m in measurements), 'sampled_peak_rss_bytes': max(m[1] for m in measurements),
                         'expected_evidence_verified': True})
            measurements.clear()
            selected = [argument for name in dataset['bundles'] for argument in ('--bundle', name)]
            response = run(*command, *selected, '--as-of', dataset['as_of'], '--limit', '5',
                           '--budget-bytes', '16384', '--max-scan', '1000', *task['filters'], code=7)
            combined = verified_evidence(response['data']['items'])
            assert combined == evidence
            assert response['data']['generations'] == generations
            assert sorted(response['data']['coverage_gaps']['unavailable_bundles']) == sorted(missing)
            assert response['page']['total'] is None
            assert response['budget']['indexed_documents_scanned'] <= 1000
            assert response['budget']['content_bytes_used'] <= 16384
            runs.append({'task': task['id'], 'mode': mode, 'strategy': 'multi_bundle', 'calls': 1,
                         'references': sorted(combined), 'missing_indexes': missing, 'coverage_complete': False,
                         'disclosed_bytes': response['budget']['content_bytes_used'],
                         'elapsed_ms': measurements[0][0], 'sampled_peak_rss_bytes': measurements[0][1],
                         'expected_evidence_verified': True})
report = {'mode': 'multi_bundle_comparison', 'as_of': dataset['as_of'], 'tasks': len(dataset['tasks']),
          'bundles': len(dataset['bundles']), 'generations': generations,
          'budget_scope': {'per_bundle': 'per request; aggregate budgets multiply by bundle count',
                           'multi_bundle': 'one shared request budget'}, 'runs': runs}
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
print(f'Cross-bundle comparison passed: {len(runs)} task/mode/strategy cases; identical evidence with {len(dataset["bundles"])} calls versus 1')
