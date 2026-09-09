#!/usr/bin/env python3
"""Mutation tests for the documentation corpus checker; no product fold."""
import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('bw_checker', Path(__file__).with_name('check-nip-bw-fixtures.py'))
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


class CorpusTests(unittest.TestCase):
    """Ensure damaged corpus inputs cannot masquerade as checked fixtures."""

    @classmethod
    def setUpClass(cls):
        cls.corpus = checker.load(checker.DEFAULT)

    def test_corpus(self):
        checker.validate(self.corpus)

    def test_mutations_rejected(self):
        mutations = {
            'unrecognized format': lambda d: d.update(format='unknown'),
            'missing record schema': lambda d: d['schemas'].pop('member-verdict'),
            'duplicate scenario': lambda d: d['cases'].append(d['cases'][0]),
            'dangling scenario input': lambda d: d['cases'][0]['input'].__setitem__(0, 'absent'),
            'missing expected step': lambda d: d['cases'][0]['expected'].pop(),
            'ID corruption': lambda d: d['events']['policy']['event'].update(id='f' * 64),
            'shape expectation lie': lambda d: d['events']['unknown-tag'].update(shape=True),
            'non-boolean download durability': lambda d: d['cases'][0]['external']['downloads'][0].update(immutable='false'),
            'non-boolean Tailnet flag': lambda d: d['cases'][0]['external']['downloads'][0].update(tailnet=None),
            'non-boolean ephemeral flag': lambda d: d['cases'][0]['external']['downloads'][0].update(ephemeral=123),
            'missing external facts': lambda d: d['cases'][0]['external'].pop('downloads'),
            'missing scenario coverage': lambda d: [c.update(coverage=[]) for c in d['cases']],
            'unmarked production identity data': lambda d: d['provenance'].update(test_only=False),
            'conflicting permutation oracle': lambda d: next(c for c in d['cases'] if c['name'] == 'late-handoff-delivery')['expected'][-1].update(projection={'handoff': 'incorrect'}),
            'invalid outcome': lambda d: d['cases'][0]['expected'][0].update(outcome='maybe'),
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label):
                data = copy.deepcopy(self.corpus)
                mutate(data)
                with self.assertRaises((ValueError, KeyError, TypeError)):
                    checker.validate(data)

    def test_strict_json(self):
        for raw in (b'{"a":1,"a":2}', b'{"a":NaN}', b'{"a":1.0}', b'{"a":1e1}', b'\xef\xbb\xbf{}', b'{}{}'):
            with self.subTest(raw=raw), tempfile.TemporaryDirectory() as tmp:
                path = Path(tmp) / 'bad.json'
                path.write_bytes(raw)
                with self.assertRaises(ValueError):
                    checker.load(path)


if __name__ == '__main__':
    unittest.main()
