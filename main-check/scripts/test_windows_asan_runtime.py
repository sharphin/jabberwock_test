"""Runtime selection regressions using synthetic PE files; no Windows execution."""
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import subprocess
import windows_asan_runtime as runtime
from test_windows_diagnostics import fixture

class RuntimeTests(unittest.TestCase):
    def test_matching_package_dll_is_selected_and_old_exports_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d); exe=root/'probe.exe'; old=root/'old.dll'; good=root/'good.dll'
            fixture(exe,imports=(runtime.DLL,'new_entry'))
            fixture(old,exported='old_entry'); fixture(good,exported='new_entry')
            discovery={'candidates':[{'path':str(p),'origin':'selected clang','exists':True} for p in (old,good)]}
            result=runtime.select(exe,discovery)
            self.assertEqual(result['selected']['path'],str(good))
            self.assertEqual(result['candidates'][0]['missing_functions'],['new_entry'])

    def test_missing_matching_runtime_does_not_fall_back_to_path(self):
        with tempfile.TemporaryDirectory() as d:
            exe=Path(d)/'probe.exe'; fixture(exe,imports=(runtime.DLL,'required'))
            result=runtime.select(exe,{'candidates':[]})
            self.assertIsNone(result['selected'])
            self.assertIn('not substituted',result['reason'])

    def test_clang_echoed_relative_basename_is_not_a_discovery(self):
        with tempfile.TemporaryDirectory() as d:
            clang=str(Path(d)/'clang.exe')
            with mock.patch.object(runtime.subprocess,'run',return_value=subprocess.CompletedProcess([],0,runtime.DLL+'\n','')):
                result=runtime.discover(clang)
            self.assertEqual(len(result['candidates']),1)
            self.assertEqual(result['candidates'][0]['origin'],'clang executable directory')

    def test_path_override_is_process_local_and_preserves_other_entries(self):
        original={'PATH':'old-path','KEEP':'yes'}
        env=runtime.environment('matching-clang-runtime', original)
        self.assertEqual(original['PATH'],'old-path')
        self.assertTrue(env['PATH'].startswith('matching-clang-runtime'))
        self.assertEqual(env['KEEP'],'yes')

if __name__=='__main__': unittest.main()
