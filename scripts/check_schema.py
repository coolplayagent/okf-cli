"""Validate real success, partial and error responses against the JSON Schema."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from jsonschema import Draft202012Validator

root = Path(__file__).resolve().parents[1]
binary = Path(sys.argv[1]).resolve()
schema = json.loads((root / 'schemas/response.schema.json').read_text(encoding='utf-8'))
Draft202012Validator.check_schema(schema)
validator = Draft202012Validator(schema)
with tempfile.TemporaryDirectory(prefix='okf-schema-') as temporary:
    workspace = Path(temporary)
    env = dict(os.environ, OKF_HOME=str(workspace / 'home'))
    bundle = workspace / 'book'
    bundle.mkdir()
    (bundle / 'note.md').write_text('---\ntype: Note\n---\nHello 认证\n', encoding='utf-8')
    cases = [
        (0, ['bundle', 'add', 'book', '--path', str(bundle)]),
        (0, ['validate', 'book']), (0, ['sync', 'book']),
        (0, ['search', 'Hello', '--bundle', 'book', '--full-text']),
        (0, ['context', '--query', 'Hello', '--bundle', 'book', '--content']),
        (0, ['read', '@book/note', '--indexed']),
        (0, ['index', 'status', 'book']), (0, ['cache', 'reset']),
        (0, ['neighbors', '@book/note']),
        (0, ['validate', 'book', '--attachments', '--anchor-style', 'headings-v1']),
        (0, ['search', 'Hello', '--bundle', 'book', '--full-text', '--type', 'Note']),
        (3, ['read', '@missing/note']),
        (7, ['sync', 'book', '--max-scan', '1']),
        (2, ['read', '@book/note', '--limit', '2']),
        (2, ['bundle', 'cache', 'book', '--compact']),
    ]
    for code, args in cases:
        result = subprocess.run([str(binary), '--format', 'json', *args], env=env,
                                cwd=workspace, capture_output=True, timeout=60)
        assert result.returncode == code, (args, result.stdout, result.stderr)
        assert not result.stderr, result.stderr
        validator.validate(json.loads(result.stdout))
print(f'Schema validated against {len(cases)} command responses')
