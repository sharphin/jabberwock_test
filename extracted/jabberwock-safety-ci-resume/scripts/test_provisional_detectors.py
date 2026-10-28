"""Fail-closed parser/catalog regression cases; no external tools are launched."""
from pathlib import Path
import unittest
from unittest import mock
import provisional_detectors as detectors
import safety_compilers

class ProvisionalTests(unittest.TestCase):
    def test_inspector_unknown_report_is_not_zero_problems(self):
        for text in ('', 'tool failed', 'Unexpected,Header\n0,0\n'):
            with self.assertRaises(ValueError): detectors.inspector_problems(text)
        self.assertEqual(detectors.inspector_problems('Problem ID,Problem Type\n'), [])
        self.assertEqual(detectors.inspector_problems('Problem ID,Problem Type\n1,Data race\n'), ['Data race'])

    def test_drmemory_requires_a_positive_clean_or_negative_signal(self):
        self.assertFalse(detectors.drmemory_result(''))
        self.assertTrue(detectors.drmemory_result('NO ERRORS FOUND'))
        self.assertFalse(detectors.drmemory_result('NO ERRORS FOUND\nError #1: UNINITIALIZED READ'))
        self.assertTrue(detectors.drmemory_result('Error #1: UNINITIALIZED READ', negative=True))
        self.assertFalse(detectors.drmemory_result('Error #1: LEAK', negative=True))

    def test_tools_missing_never_become_success_probes(self):
        with mock.patch.object(detectors, 'executable', return_value=None):
            _, probes = safety_compilers.catalog('windows', 1, Path('work'))
        for subject in ('jwc','rust'):
            for kind in ('thread','memory','undefined','llvm-verifier'):
                p=next(p for p in probes if p['name']==subject+'-'+kind)
                self.assertIn('unavailable', p['unavailable_reason'])
                self.assertIn('provisional_detector',p)

    def test_each_compiler_requires_its_own_negative_and_complete_case_ids(self):
        with mock.patch.object(detectors,'executable',return_value='configured-tool'):
            _, probes=safety_compilers.catalog('windows',1,Path('work'))
        for subject in ('jwc','rust'):
            for kind in ('thread','memory','undefined','llvm-verifier'):
                p=next(p for p in probes if p['name']==subject+'-'+kind)
                self.assertEqual(p['expected_subject'],subject)
                self.assertIn('detector-negative',p['required_case_ids'])
                self.assertEqual(len(p['required_case_ids']),len(set(p['required_case_ids'])))
                self.assertIn('scripts/check_external_detector.py',p['cmd'])
                self.assertTrue(p['provisional_detector']['replacement'])

if __name__=='__main__': unittest.main()
