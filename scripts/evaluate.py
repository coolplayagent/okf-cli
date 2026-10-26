"""Measure retrieval and citation fidelity with a fixed evidence task set.

Usage: python scripts/evaluate.py BINARY [OUTPUT.json]
Latency includes process startup. No model is called and no answer quality is inferred.
"""
import hashlib
import argparse
import json
import os
from pathlib import Path
import tempfile
from measure import run_measured

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('output', nargs='?', type=Path)
parser.add_argument('--scale', type=int, default=0, help='Add up to 10000 irrelevant 1-KiB documents')
parser.add_argument('--modes', nargs='+', choices=('metadata', 'full_text', 'context'), default=['metadata', 'full_text', 'context'])
args = parser.parse_args()
if not 0 <= args.scale <= 10000:
    parser.error('--scale must be between 0 and 10000')
binary = args.binary.resolve()
dataset = json.loads((root / 'fixtures/evaluation/tasks.json').read_text(encoding='utf-8'))
rows = []
with tempfile.TemporaryDirectory(prefix='okf-eval-') as temporary:
    workspace = Path(temporary)
    env = dict(os.environ, OKF_HOME=str(workspace / 'home'))
    bundle = workspace / 'bundle'
    bundle.mkdir()
    for path, raw in dataset['documents'].items():
        (bundle / path).write_bytes(raw.encode('utf-8'))
    for i in range(args.scale):
        (bundle / f'filler-{i:05}.md').write_text('---\ntype: Filler\n---\n' + 'unrelated material ' * 55, encoding='utf-8')

    def run(*args):
        result, elapsed, peak = run_measured([str(binary), '--format', 'json', *args], cwd=workspace, env=env)
        assert result.returncode in (0, 7), (args, result.stdout, result.stderr)
        assert not result.stderr
        return json.loads(result.stdout), elapsed, peak

    run('bundle', 'add', 'eval', '--path', str(bundle))
    preparation = {}
    for operation, command in (
        ('sync', ['sync', 'eval', '--max-scan', '100000', '--timeout-ms', '120000']),
        ('index_status', ['index', 'status', 'eval']),
        ('indexed_read', ['read', '@eval/' + next(iter(dataset['documents'])), '--indexed']),
    ):
        response, elapsed, peak = run(*command)
        assert response['status'] == 'ok', response
        preparation[operation] = {'elapsed_ms': elapsed, 'sampled_peak_rss_bytes': peak}
    assert response['data']['document']['text'].encode('utf-8') == next(iter(dataset['documents'].values())).encode('utf-8')
    for task in dataset['tasks']:
        for mode in dict.fromkeys(args.modes):
            command = ['context', '--query', task['query'], '--content'] if mode == 'context' else ['search', task['query']]
            if mode == 'full_text':
                command += ['--full-text']
            response, elapsed, peak = run(*command, '--bundle', 'eval', '--limit', '5', '--budget-bytes', '16384', '--max-scan', '100000', '--timeout-ms', '120000')
            items = response['data']['items']
            paths = {item['id'] + '.md' for item in items}
            expected = set(task['evidence'])
            evidence_found = 0
            read_calls = 0
            disclosed = response['budget']['content_bytes_used']
            for item in items:
                path = item['id'] + '.md'
                original = dataset['documents'][path].encode('utf-8')
                if 'snippet' in item:
                    snippet = item['snippet']
                    text = snippet['text']
                    assert original[snippet['byte_start']:snippet['byte_end']] == text.encode('utf-8')
                    assert item['content_sha256'] == hashlib.sha256(original).hexdigest()
                else:
                    read, latency, read_peak = run('read', item['ref'])
                    peak = max(peak, read_peak)
                    elapsed += latency
                    read_calls += 1
                    text = read['data']['document']['text']
                    assert text.encode('utf-8') == original
                    disclosed += len(original)
                if path in task['evidence'] and task['evidence'][path] in text:
                    evidence_found += 1
            rows.append({'task': task['id'], 'mode': mode, 'recall_at_5': len(paths & expected) / len(expected) if expected else None,
                         'evidence_recall': evidence_found / len(expected) if expected else None,
                         'no_answer_correct': not paths if not expected else None,
                         'citations_verified': len(items), 'calls': 1 + read_calls,
                         'disclosed_bytes': disclosed, 'elapsed_ms': round(elapsed, 3), 'sampled_peak_rss_bytes': peak})
            if mode in ('full_text', 'context'):
                correct = rows[-1]['recall_at_5'] == 1 and rows[-1]['evidence_recall'] == 1 if expected else not paths
                assert correct, rows[-1]
def mean(mode, metric):
    values = [row[metric] for row in rows if row['mode'] == mode and row[metric] is not None]
    return round(sum(values) / len(values), 4)
report = {'dataset': 'fixtures/evaluation/tasks.json', 'tasks': len(dataset['tasks']),
          'documents': len(dataset['documents']) + args.scale,
          'preparation': preparation,
          'memory_measurement': 'CLI RSS sampled every 2 ms; brief peaks may be missed; per task maximum across its CLI calls',
          'summary': {mode: {metric: mean(mode, metric)
                            for metric in ('recall_at_5', 'evidence_recall', 'no_answer_correct', 'calls', 'disclosed_bytes', 'elapsed_ms', 'sampled_peak_rss_bytes')}
                       for mode in dict.fromkeys(args.modes)}, 'runs': rows}
output = json.dumps(report, ensure_ascii=False, indent=2) + '\n'
if args.output:
    destination = args.output
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(output, encoding='utf-8')
print(json.dumps(report['summary'], ensure_ascii=False, indent=2))
