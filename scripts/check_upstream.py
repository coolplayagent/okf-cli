"""Verify pinned fixtures and report upstream changes without rewriting the baseline.

Network access is explicit with --online. Changed specs and sample files require
human classification against the conformance matrix, not automatic acceptance.
"""
import argparse
import hashlib
import json
from pathlib import Path
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
SPEC_PATHS = {'google':'SPEC.md', 'skills':'skills/okf/reference/SPEC.md',
              'ruby':'gems/okf/lib/okf/skill/reference/SPEC.md'}


def fetch(url):
    request = urllib.request.Request(url,headers={'User-Agent':'okf-cli-upstream-audit'})
    with urllib.request.urlopen(request,timeout=60) as response:
        return json.load(response)


def inventory(tree, bundle, spec):
    if tree.get('truncated'):
        raise ValueError('upstream tree truncated; refusing to report complete coverage')
    result = {entry['path']:entry['sha'] for entry in tree['tree'] if entry['type']=='blob'
            and (entry['path']==spec or (entry['path'].startswith(bundle+'/') and entry['path'].endswith('.md')))}
    if spec not in result or not any(path.startswith(bundle+'/') for path in result):
        raise ValueError('expected specification or bundle is missing; upstream layout needs review')
    return result


def compare(before, after):
    return {'added':sorted(after.keys()-before.keys()),'removed':sorted(before.keys()-after.keys()),
            'changed':sorted(p for p in before.keys() & after.keys() if before[p]!=after[p])}


def audit(online, fetcher=fetch):
    manifest = json.loads((ROOT/'fixtures/upstream/manifest.json').read_text(encoding='utf-8'))
    results = []
    for source in manifest:
        for file in source['files']:
            raw = (ROOT/'fixtures/upstream'/file['path']).read_bytes()
            if hashlib.sha256(raw).hexdigest()!=file['sha256']:
                raise ValueError(f'fixture modified: {file["path"]}')
        result = {'name':source['name'],'repository':source['repository'],'baseline':source['commit'],
                  'local_hashes_valid':True,'upstream_checked':online,'changes':None}
        if online:
            api = source['repository'].replace('https://github.com/','https://api.github.com/repos/')
            # Resolve once, then read both immutable trees to avoid a moving-head comparison.
            head = fetcher(api+'/commits/HEAD')['sha']
            spec = SPEC_PATHS[source['name']]
            before = inventory(fetcher(api+'/git/trees/'+source['commit']+'?recursive=1'),source['bundle'],spec)
            after = inventory(fetcher(api+'/git/trees/'+head+'?recursive=1'),source['bundle'],spec)
            result.update(head=head, spec_path=spec, changes=compare(before,after),
                          spec_changed=before.get(spec)!=after.get(spec))
            result['review_required'] = any(result['changes'].values())
        results.append(result)
    return {'sources':results, 'baseline_rewritten':False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output',type=Path)
    parser.add_argument('--online',action='store_true')
    args = parser.parse_args()
    report = audit(args.online)
    args.output.parent.mkdir(parents=True,exist_ok=True)
    args.output.write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    print(f'Upstream audit passed: {len(report["sources"])} baselines; online={args.online}')


if __name__ == '__main__':
    main()
