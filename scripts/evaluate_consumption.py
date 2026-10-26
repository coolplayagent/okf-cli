"""Replay twelve CLI consumption tasks; evaluate evidence and coverage, not model answers."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

binary, output = Path(sys.argv[1]).resolve(), Path(sys.argv[2])
rows = []
with tempfile.TemporaryDirectory(prefix='okf-consumption-') as temporary:
    work = Path(temporary)
    env = dict(os.environ, OKF_HOME=str(work / 'home'))
    originals = {}
    for name, text in (('primary', 'Use durable queues.'), ('secondary', 'Use direct calls.')):
        directory = work / name
        docs = {
            'a/background.md': 'Background only.',
            'decisions/transport.md': text,
            'runbooks/current.md': 'Recovery: replay the queue.',
            'runbooks/old.md': 'Recovery: discard the queue.',
        }
        for path, body in docs.items():
            fields = 'type: Note\ntitle: transport recovery\n'
            if path.endswith('current.md'):
                fields += 'stale_after: "2040-01-01T00:00:00Z"\nverified:\n  by: human:operator\n  at: "2025-01-01T00:00:00Z"\n'
            elif path.endswith('old.md'):
                fields += 'stale_after: "2020-01-01T00:00:00Z"\n'
            raw = ('---\n' + fields + '---\n' + body + '\n').encode()
            target = directory / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(raw)
            originals[f'@{name}/{path.removesuffix(".md")}'] = raw

    trace = []
    def call(*args, code=0):
        process = subprocess.run([str(binary), '--format', 'json', *args], cwd=work,
                                 env=env, capture_output=True, timeout=30)
        assert process.returncode == code, (args, process.stdout, process.stderr)
        assert not process.stderr
        response = json.loads(process.stdout)
        if args[0] == 'read' and code == 0:
            document = response['data']['document']
            raw = originals[args[1]]
            assert document['content_sha256'] == hashlib.sha256(raw).hexdigest()
            assert document['text'].encode() == raw[document['byte_start']:document['byte_end']]
        trace.append({'arguments': list(args), 'exit_code': code, 'response': response})
        return response

    def evidence(response):
        refs = []
        for item in response['data']['items']:
            ref = item['ref']
            raw = originals[ref]
            assert item['content_sha256'] == hashlib.sha256(raw).hexdigest()
            if 'snippet' in item:
                snippet = item['snippet']
                assert raw[snippet['byte_start']:snippet['byte_end']] == snippet['text'].encode()
            refs.append(ref)
        return refs

    def finish(name, expected, response=None, decision='evidence_available'):
        refs = evidence(response) if response else []
        if expected is not None:
            assert set(refs) == set(expected), (name, refs, expected)
        rows.append({'task': name, 'passed': True, 'decision': decision,
                     'references': refs, 'calls': len(trace),
                     'disclosed_bytes': sum(t['response']['budget'].get('content_bytes_used', 0) for t in trace),
                     'trace': list(trace)})
        trace.clear()

    for name in ('primary', 'secondary'):
        call('bundle', 'add', name, '--path', str(work / name))
        call('sync', name)
    call('bundle', 'add', 'missing', '--path', str(work / 'primary'))
    trace.clear()
    call('context')
    call('browse', '@primary/decisions')
    doc = call('read', '@primary/decisions/transport')['data']['document']
    assert doc['text'].encode() == originals['@primary/decisions/transport']
    finish('discover-browse-read', None)
    response = call('search', 'transport', '--bundle', 'primary', '--dir', 'decisions')
    for item in response['data']['items']:
        call('read', item['ref'])
    finish('metadata-then-read', ['@primary/decisions/transport'], response)
    response = call('search', 'durable', '--bundle', 'primary', '--full-text')
    finish('body-evidence', ['@primary/decisions/transport'], response)
    response = call('context', '--query', 'transport', '--bundle', 'primary', '--bundle', 'secondary', '--dir', 'decisions', '--content')
    finish('conflicting-sources-retained', ['@primary/decisions/transport', '@secondary/decisions/transport'], response)
    response = call('search', 'Recovery', '--bundle', 'primary', '--dir', 'runbooks', '--full-text', '--expiry', 'current', '--as-of', '2030-01-01T00:00:00Z')
    finish('exclude-expired-evidence', ['@primary/runbooks/current'], response)
    response = call('search', 'Recovery', '--bundle', 'primary', '--full-text', '--trust', 'human-reviewed')
    finish('reviewed-evidence', ['@primary/runbooks/current'], response)
    for name in ('primary', 'secondary'):
        (work / name).rename(work / (name + '-offline'))
    response = call('context', '--query', 'durable', '--bundle', 'primary', '--content')
    finish('offline-excerpts', ['@primary/decisions/transport'], response)
    response = call('search', 'durable', '--bundle', 'primary', '--dir', 'decisions', '--full-text', '--max-scan', '1')
    assert response['budget']['indexed_documents_scanned'] == 1
    finish('one-document-scoped-budget', ['@primary/decisions/transport'], response)
    response = call('search', 'durable', '--bundle', 'primary', '--full-text', '--max-scan', '1', code=7)
    assert response['page']['total'] is None
    finish('partial-zero-is-inconclusive', [], response, 'inconclusive')
    response = call('search', 'absent-evidence', '--bundle', 'primary', '--full-text')
    assert response['page']['total'] == 0
    finish('complete-zero-within-snapshot', [], response, 'no_matching_evidence')
    response = call('search', 'durable', '--bundle', 'primary', '--bundle', 'missing', '--full-text', code=7)
    assert response['page']['total'] is None
    assert response['data']['coverage_gaps']['unavailable_bundles'] == ['missing']
    finish('missing-index-keeps-evidence', ['@primary/decisions/transport'], response, 'inconclusive')
    previous = response['data']['generations']['primary']
    (work / 'primary-offline').rename(work / 'primary')
    (work / 'primary/decisions/transport.md').write_text('---\ntype: Note\n---\nChanged decision\n', encoding='utf-8')
    call('sync', 'primary')
    error = call('read', '@primary/decisions/transport', '--indexed', '--generation', previous, code=6)
    assert error['data']['error']['code'] == 'REVISION_CHANGED'
    finish('stale-generation-rejected', None, decision='restart_required')

output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(json.dumps({'protocol': 'deterministic-cli-v1', 'model': None,
                             'answer_correctness': None, 'unsupported_claim_rate': None,
                             'tasks': rows}, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
print(f'Consumption replay passed: {len(rows)} tasks with complete CLI traces')
