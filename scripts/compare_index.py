"""Compare two built binaries on the same 1000-document, ~32 MB index."""
import argparse
import json
import os
from pathlib import Path
import tempfile

from measure import run_measured

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('before', type=Path)
parser.add_argument('after', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
before, after = args.before.resolve(), args.after.resolve()
with tempfile.TemporaryDirectory(prefix='okf-index-comparison-') as temporary:
    work = Path(temporary)
    env = dict(os.environ, OKF_HOME=str(work / 'home'))
    bundle = work / 'bundle'
    bundle.mkdir()
    raw = '---\ntype: Note\n---\nneedle ' + 'irrelevant material ' * 1600
    for i in range(1000):
        (bundle / f'note-{i:04}.md').write_text(raw, encoding='utf-8')

    def call(binary, arguments):
        result, elapsed, peak = run_measured([str(binary), '--format', 'json', *arguments], cwd=work, env=env)
        if result.returncode not in (0, 7) or result.stderr:
            raise AssertionError((arguments, result.stdout, result.stderr))
        return json.loads(result.stdout), {'elapsed_ms': elapsed, 'sampled_peak_rss_bytes': peak}

    call(after, ['bundle', 'add', 'book', '--path', str(bundle)])
    sync, _ = call(after, ['sync', 'book', '--max-scan', '10000'])
    assert sync['data']['published'] is True
    report = {'documents': 1000, 'source_bytes': len(raw.encode('utf-8')) * 1000,
              'before': call(before, ['build-info'])[0]['data'],
              'after': call(after, ['build-info'])[0]['data'], 'runs': []}
    for operation, arguments in (
        ('index_status', ['index', 'status', 'book']),
        ('indexed_read', ['read', '@book/note-0000', '--indexed']),
        ('bounded_search', ['search', 'needle', '--bundle', 'book', '--full-text', '--max-scan', '10']),
    ):
        for iteration in range(3):
            old, old_measure = call(before, arguments)
            new, new_measure = call(after, arguments)
            if operation == 'bounded_search':
                assert [(x['ref'], x['snippet'], x['generation']) for x in old['data']['items']] == [
                    (x['ref'], x['snippet'], x['generation']) for x in new['data']['items']]
            else:
                assert old['data'] == new['data']
            report['runs'].append({'operation': operation, 'iteration': iteration,
                                   'before': old_measure, 'after': new_measure})
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
print('Index comparison passed: identical content, versions and selected evidence')
