"""Check directory selection before scan allocation, including literal path boundaries."""
from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
from jsonschema import Draft202012Validator

binary = Path(sys.argv[1]).resolve()
schema = json.loads((Path(__file__).resolve().parents[1] / 'schemas/response.schema.json').read_text(encoding='utf-8'))
validator = Draft202012Validator(schema)
with tempfile.TemporaryDirectory(prefix='okf-scope-') as temporary:
    work = Path(temporary)
    env = dict(os.environ, OKF_HOME=str(work / 'home'))
    root = work / 'book'
    paths = ['a/noise.md', 'services/auth.md', 'services/nested/key.md',
             'services-other/wrong.md', '认证%_/note.md', '认证XX/wrong.md']
    for path in paths:
        file = root / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text('---\ntype: Note\ntitle: needle\n---\nneedle evidence\n', encoding='utf-8')

    def call(*args, code=0):
        result = subprocess.run([str(binary), '--format', 'json', *args], cwd=work,
                                env=env, capture_output=True, timeout=30)
        assert result.returncode == code, (args, result.stdout, result.stderr)
        assert not result.stderr
        response = json.loads(result.stdout)
        validator.validate(response)
        return response

    call('bundle', 'add', 'book', '--path', str(root))
    call('sync', 'book')
    other = work / 'other'
    (other / 'services').mkdir(parents=True)
    (other / 'services/extra.md').write_text('---\ntype: Note\ntitle: needle\n---\nneedle\n', encoding='utf-8')
    call('bundle', 'add', 'other', '--path', str(other))
    call('sync', 'other')
    for mode in (['search', 'needle'], ['context', '--query', 'needle'],
                 ['search', 'needle', '--full-text'], ['context', '--query', 'needle', '--content']):
        response = call(*mode, '--bundle', 'book', '--dir', 'services/')
        assert response['data']['directory'] == 'services'
        assert response['bundles'][0]['coverage']['scope'] == 'services'
        assert {i['id'] for i in response['data']['items']} == {'services/auth', 'services/nested/key'}
        response = call(*mode, '--bundle', 'book', '--dir', '认证%_')
        assert [i['id'] for i in response['data']['items']] == ['认证%_/note']
    for directory in ('../services', '/services', '.', '', 'services//', 'C:/services'):
        call('search', 'needle', '--bundle', 'book', '--dir', directory, code=2)
    call('context', '--bundle', 'book', '--dir', 'services', code=2)
    # Outside-scope damage must neither consume the scope's scan quota nor be loaded.
    with closing(sqlite3.connect(work / 'home/content.sqlite')) as connection:
        connection.execute("UPDATE documents SET raw='damaged' WHERE path='a/noise.md'")
        connection.commit()
    root.rename(work / 'offline')
    response = call('search', 'needle', '--full-text', '--bundle', 'book', '--bundle', 'other',
                    '--dir', 'services', '--max-scan', '3')
    assert response['page']['total'] == 3
    assert response['budget']['indexed_documents_scanned'] == 3
    assert set(response['data']['generations']) == {'book', 'other'}
    for mode in (['search', 'needle', '--full-text'], ['context', '--query', 'needle', '--content']):
        response = call(*mode, '--bundle', 'book', '--dir', 'services', '--max-scan', '2')
        assert response['page']['total'] == 2
        assert response['budget']['indexed_documents_scanned'] == 2
        response = call(*mode, '--bundle', 'book', '--dir', 'services', '--max-scan', '1', code=7)
        assert response['page']['total'] is None
        assert response['budget']['indexed_documents_scanned'] == 1
        response = call(*mode, '--bundle', 'book', '--dir', 'absent', '--max-scan', '1')
        assert response['page']['total'] == 0 and response['data']['items'] == []
print('Directory selection passed: four modes, path boundaries, offline reads and scan budgets')
