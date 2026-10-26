"""Regression checks for delivery gates, upstream changes and candidate budgets."""
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch
from check_runtime import inspect, pe_imports
from check_upstream import compare, inventory
from evaluate_candidates import query


class SupportChecks(unittest.TestCase):
    def test_candidate_zero_budgets_never_claim_complete_coverage(self):
        with tempfile.TemporaryDirectory() as temporary:
            database = Path(temporary) / 'prototype.sqlite'
            connection = sqlite3.connect(database)
            try:
                connection.executescript("CREATE TABLE docs(path TEXT); INSERT INTO docs VALUES('note.md'); CREATE TABLE postings(gram TEXT,path TEXT); INSERT INTO postings VALUES('ter','note.md');")
                connection.commit()
            finally:
                connection.close()
            for text, limit, work_limit, engine in (('term', 0, 10, 'scan'),
                                                   ('te', 0, 10, 'trigram'),
                                                   ('term', 10, 0, 'trigram')):
                with self.subTest(engine=engine, limit=limit, work_limit=work_limit):
                    result = query(database, text, limit, work_limit, engine)
                    self.assertFalse(result['complete'])
                    self.assertEqual(result['matches'], [])
                    self.assertEqual(result['body_checks'], 0)
                    self.assertLessEqual(result['posting_rows'], work_limit)

    def test_candidate_invalid_inputs_rejected_before_opening_database(self):
        for text, limit, work_limit in ((None, 1, 1), ('  ', 1, 1),
                                       ('term', -1, 1), ('term', 1, -1)):
            with self.subTest(text=text, limit=limit, work_limit=work_limit):
                with patch('evaluate_candidates.sqlite3.connect') as connect:
                    with self.assertRaises(ValueError):
                        query(Path('unused.sqlite'), text, limit, work_limit)
                    connect.assert_not_called()

    def test_upstream_changes_preserve_additions_removals_and_edits(self):
        self.assertEqual(compare({'old':'a','edited':'b','same':'c'}, {'new':'a','edited':'d','same':'c'}),
                         {'added':['new'],'removed':['old'],'changed':['edited']})

    def test_upstream_requires_complete_tree_and_expected_paths(self):
        for tree in ({'truncated':True}, {'tree':[]},
                     {'tree':[{'type':'blob','path':'SPEC.md','sha':'a'}]}):
            with self.assertRaises(ValueError):
                inventory(tree,'bundle','SPEC.md')
        result = inventory({'tree':[{'type':'blob','path':p,'sha':'a'} for p in
                                    ('SPEC.md','bundle/note.md','bundle-extra/note.md','code.py')]},'bundle','SPEC.md')
        self.assertEqual(set(result), {'SPEC.md','bundle/note.md'})

    def test_linux_accepts_baseline_and_records_libraries(self):
        with patch('check_runtime.subprocess.check_output',side_effect=[
                'Name: GLIBC_2.2.5\nName: GLIBC_2.35\n','(NEEDED) Shared library: [libc.so.6]']):
            report = inspect(Path('okf'),'x86_64-unknown-linux-gnu')
        self.assertEqual(report['glibc_max'],'2.35')
        self.assertEqual(report['libraries'],['libc.so.6'])

    def test_linux_rejects_missing_or_newer_glibc(self):
        for text in ('Name: GLIBC_2.39','no version information'):
            with patch('check_runtime.subprocess.check_output',side_effect=[text,'']):
                with self.assertRaises(ValueError):
                    inspect(Path('okf'),'x86_64-unknown-linux-gnu')

    def test_macos_baseline_and_excessive_deployment_target(self):
        for version in ('14.0','14.0.0'):
            with patch('check_runtime.subprocess.check_output',side_effect=[f'minos {version}','okf:\n /usr/lib/libSystem.B.dylib (version)']):
                self.assertEqual(inspect(Path('okf'),'aarch64-apple-darwin')['deployment_targets'],[version])
        for text in ('minos 15.0','missing'):
            with patch('check_runtime.subprocess.check_output',side_effect=[text,'']):
                with self.assertRaises(ValueError):
                    inspect(Path('okf'),'aarch64-apple-darwin')

    def test_invalid_pe_is_rejected(self):
        with self.assertRaises(ValueError):
            pe_imports(bytes(128))


if __name__ == '__main__':
    unittest.main()
