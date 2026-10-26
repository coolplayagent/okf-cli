"""Check evaluation preparation cannot modify or inherit parent project bindings."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib

binary = Path(sys.argv[1]).resolve()
prepare = Path(__file__).with_name('prepare_agent_evaluation.py').resolve()
with tempfile.TemporaryDirectory(prefix='okf-agent-prepare-') as temporary:
    parent = Path(temporary)
    (parent / '.git').mkdir()
    config = parent / '.okf-cli/config.toml'
    config.parent.mkdir()
    original = b'bundles = []\n# preserve parent project\n'
    config.write_bytes(original)
    output = parent / 'evaluation'
    subprocess.run([sys.executable, str(prepare), str(binary), str(output)], check=True, timeout=120)
    assert config.read_bytes() == original
    for corpus, expected in [('single', {'book'}), ('multi', {'handbook', 'operations', 'legacy', 'unsynced'})]:
        project = output / corpus
        data = tomllib.loads((project / '.okf-cli/config.toml').read_text(encoding='utf-8'))
        assert {b['name'] for b in data['bundles']} == expected
        assert all((project / b['root']).resolve().is_relative_to(project.resolve()) for b in data['bundles'])
        env = dict(os.environ, OKF_HOME=str(project / 'home'))
        result = json.loads(subprocess.check_output([str(binary), '--format', 'json', 'bundle', 'list'], cwd=project, env=env))
        assert len(result['data']['items']) == len(expected)
    packet = json.loads((output / 'tasks.json').read_text(encoding='utf-8'))
    grading = json.loads((output / 'grading.json').read_text(encoding='utf-8'))
    manifest = json.loads((output / 'manifest.json').read_text(encoding='utf-8'))
    assert manifest['binary_sha256'] == hashlib.sha256(binary.read_bytes()).hexdigest()
    assert not manifest['model_run']
    assert len(packet['tasks']) == len(grading['tasks']) == 6
    assert all('required_claims' not in t and 'expected_refs' not in t for t in packet['tasks'])
    assert all(set(t['expected_refs']) <= grading['source_hashes'][t['corpus']].keys() for t in grading['tasks'])
    before = {p.name: p.read_bytes() for p in output.glob('*.json')}
    rejected = subprocess.run([sys.executable, str(prepare), str(binary), str(output)], capture_output=True, timeout=120)
    assert rejected.returncode == 2
    assert before == {p.name: p.read_bytes() for p in output.glob('*.json')}
    assert config.read_bytes() == original
print('Agent preparation passed: isolated bindings, blind tasks, hashes and overwrite refusal')
