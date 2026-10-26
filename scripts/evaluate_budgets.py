"""Measure evidence loss under shared budgets on unequal, offline bundles.

The complete-scan baseline establishes expected evidence; constrained runs record
losses rather than assuming that partial scans have full recall.
"""
import argparse
import hashlib
import itertools
import json
import os
from pathlib import Path
import tempfile

from measure import run_measured

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
binary = args.binary.resolve()
bundles = {}
for name, size in (('large', 60), ('medium', 8), ('small', 3)):
    docs = {}
    for i in range(size):
        body = 'ordinary background material'
        tags = '[]'
        if i == size - 1:
            body = f'late-evidence {name} authoritative tail'
            tags = '[selected]'
        elif i == 0:
            body = 'duplicate-evidence shared recovery procedure'
        docs[f'note-{i:03}.md'] = f'---\ntype: Note\ntags: {tags}\n---\n{body}\n'
    bundles[name] = docs
tasks = [
    {'id': 'tail-evidence', 'query': 'late-evidence', 'filters': [],
     'expected': [f'@{name}/note-{len(docs)-1:03}' for name, docs in bundles.items()]},
    {'id': 'filtered-sparse-evidence', 'query': 'late-evidence', 'filters': ['--tag', 'selected'],
     'expected': [f'@{name}/note-{len(docs)-1:03}' for name, docs in bundles.items()]},
    {'id': 'duplicate-source-pressure', 'query': 'duplicate-evidence', 'filters': [],
     'expected': [f'@{name}/note-000' for name in bundles]},
    {'id': 'no-answer', 'query': 'absent-evidence', 'filters': [], 'expected': []},
]
runs = []
with tempfile.TemporaryDirectory(prefix='okf-budget-evaluation-') as temporary:
    work = Path(temporary)
    env = dict(os.environ, OKF_HOME=str(work / 'home'))
    generations = {}

    def call(arguments):
        process, elapsed, peak = run_measured([str(binary), '--format', 'json', *arguments], cwd=work, env=env)
        assert process.returncode in (0, 7), (arguments, process.stdout, process.stderr)
        assert not process.stderr
        return json.loads(process.stdout), elapsed, peak

    for name, docs in bundles.items():
        directory = work / name
        directory.mkdir()
        for path, raw in docs.items():
            (directory / path).write_bytes(raw.encode('utf-8'))
        call(['bundle', 'add', name, '--path', str(directory)])
        response, _, _ = call(['sync', name, '--max-scan', '1000'])
        assert response['status'] == 'ok'
        generations[name] = response['data']['generation']
        directory.rename(work / (name + '-offline'))

    def query(task, mode, scan, limit, budget, order):
        command = ['search', task['query'], '--full-text'] if mode == 'full_text' else ['context', '--query', task['query'], '--content']
        selected = [argument for name in order for argument in ('--bundle', name)]
        return call([*command, *selected, *task['filters'], '--max-scan', str(scan),
                     '--limit', str(limit), '--budget-bytes', str(budget), '--as-of', '2030-01-01T00:00:00Z'])

    def evidence(response):
        result = {}
        for item in response['data']['items']:
            name, path = item['ref'][1:].split('/', 1)
            raw = bundles[name][path + '.md'].encode('utf-8')
            snippet = item['snippet']
            assert raw[snippet['byte_start']:snippet['byte_end']] == snippet['text'].encode('utf-8')
            assert hashlib.sha256(raw).hexdigest() == item['content_sha256']
            assert item['generation'] == generations[name]
            result[item['ref']] = snippet['text']
        return result

    for task, mode in itertools.product(tasks, ('full_text', 'context')):
        baseline, _, _ = query(task, mode, 100, 100, 65536, bundles)
        expected = evidence(baseline)
        assert set(expected) == set(task['expected']) and baseline['status'] == 'ok'
        for scan, limit, budget in itertools.product((1, 3, 10, 100), (1, 5), (512, 4096)):
            response, elapsed, peak = query(task, mode, scan, limit, budget, bundles)
            selected = evidence(response)
            reversed_response, _, _ = query(task, mode, scan, limit, budget, reversed(bundles))
            assert response['data']['items'] == reversed_response['data']['items']
            assert response['budget'] == reversed_response['budget']
            assert response['data']['generations'] == generations
            assert all(expected.get(ref) == text for ref, text in selected.items())
            assert len(selected) <= limit
            assert response['budget']['content_bytes_used'] <= budget
            assert response['budget']['indexed_documents_scanned'] <= scan
            complete = response['page']['total'] is not None
            if scan < sum(map(len, bundles.values())):
                assert not complete and response['status'] == 'partial'
            else:
                assert complete
            if scan == 100 and limit == 5 and budget == 4096:
                assert selected == expected
            runs.append({'task': task['id'], 'mode': mode, 'scan_limit': scan,
                         'result_limit': limit, 'byte_limit': budget,
                         'evidence_recall': len(selected) / len(expected) if expected else None,
                         'source_coverage': len({ref.split('/')[0] for ref in selected}) / 3 if expected else None,
                         'no_answer_confirmed': not selected and complete if not expected else None,
                         'scan_complete': complete, 'references': list(selected),
                         'disclosed_bytes': response['budget']['content_bytes_used'],
                         'scanned': response['budget']['indexed_documents_scanned'],
                         'elapsed_ms': elapsed, 'sampled_peak_rss_bytes': peak})
report = {'strategy': 'current_round_robin_path_order', 'bundle_documents': {n: len(d) for n,d in bundles.items()},
          'tasks': tasks, 'runs': runs,
          'notes': 'Baseline checks exact evidence; constrained recall is measured, not required to equal complete scans. Reversed bundle order must preserve selection.'}
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
print(f'Budget evaluation passed: {len(runs)} cases and reversed-order checks; constrained recall recorded')
