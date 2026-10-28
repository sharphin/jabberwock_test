"""Synthetic PE and report integration regressions. Never loads fixture DLLs."""
import contextlib
import io
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import windows_pe
import safety_report


def fixture(path, *, imports=None, exported=None, forwarder=None):
    data = bytearray(0x1200)
    def u16(at, x): struct.pack_into('<H', data, at, x)
    def u32(at, x): struct.pack_into('<I', data, at, x)
    def string(at, s): data[at:at+len(s)+1] = s.encode()+b'\0'
    data[:2] = b'MZ'; u32(0x3c, 0x80); data[0x80:0x84] = b'PE\0\0'
    u16(0x84, 0x8664); u16(0x86, 1); u16(0x94, 240)
    u16(0x98, 0x20b); u32(0x98+60, 0x200); u32(0x98+108, 16)
    section = 0x98+240
    u32(section+8, 0x1000); u32(section+12, 0x1000); u32(section+16, 0x1000); u32(section+20, 0x200)
    if exported:
        u32(0x98+112, 0x1100); u32(0x98+116, 0x100)
        # export directory at raw 0x300; one function, ordinal 1, one name
        for i,x in enumerate((1,1,1,0x1140,0x1148,0x1150)): u32(0x310+4*i,x)
        u32(0x340, 0x1160 if forwarder else 0x1500); u32(0x348,0x1180); u16(0x350,0)
        string(0x380,exported)
        if forwarder: string(0x360,forwarder)
    if imports:
        dll,symbol=imports
        u32(0x98+120,0x1200); u32(0x98+124,40)
        u32(0x400,0x1250); u32(0x40c,0x1300); u32(0x410,0x1250)
        string(0x500,dll)
        struct.pack_into('<Q',data,0x450,(1<<63)|int(symbol[1:]) if symbol.startswith('#') else 0x1280)
        if not symbol.startswith('#'): string(0x482,symbol)
    path.write_bytes(data)

class DiagnosticTests(unittest.TestCase):
    def test_actual_loaded_path_and_missing_function_are_recorded(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d); exe=root/'probe.exe'; dll=root/'runtime.dll'
            fixture(exe,imports=('runtime.dll','required')); fixture(dll,exported='old_version')
            result=windows_pe.diagnose(exe,{'modules':[{'path':str(dll)}]})
            self.assertEqual(result['missing_exports'][0]['function'],'required')
            self.assertEqual(result['missing_exports'][0]['loaded_path'],str(dll))
            self.assertEqual(result['modules'][1]['machine'],'0x8664')

    def test_ordinal_and_forwarder_are_resolved(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d); exe=root/'probe.exe'; dll=root/'runtime.dll'; target=root/'target.dll'
            fixture(exe,imports=('runtime.dll','#1')); fixture(dll,exported='entry',forwarder='target.real'); fixture(target,exported='real')
            result=windows_pe.diagnose(exe,{'modules':[{'path':str(dll)},{'path':str(target)}]})
            self.assertFalse(result['missing_exports'])
            self.assertFalse(result['unresolved_imports'])

    def test_no_trace_does_not_invent_a_loaded_path(self):
        with tempfile.TemporaryDirectory() as d:
            exe=Path(d)/'probe.exe'; fixture(exe,imports=('missing.dll','required'))
            result=windows_pe.diagnose(exe,{})
            self.assertFalse(result['missing_exports'])
            self.assertEqual(result['unresolved_imports'][0]['function'],'required')

    def test_diagnostics_survive_single_file_report_embedding(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d); path=root/'diagnostic.json'
            probe=safety_report.safety.probe('windows-asan-preflight',[],[sys.executable])
            probe['case_report']=str(path)
            details={'results':[{'id':'startup','status':'fail','diagnostics':{'loader_trace':{'modules':[{'path':'actual.dll'}]},'pe_analysis':{'missing_exports':[{'function':'required'}]}}}], 'infrastructure_errors':['ENTRYPOINT_NOT_FOUND']}
            def execute(*args,**kwargs):
                path.write_text(json.dumps(details))
                return subprocess.CompletedProcess(args[0],2,'','')
            with mock.patch.object(safety_report,'execute_probe',side_effect=execute), contextlib.redirect_stdout(io.StringIO()):
                result=safety_report.run_probe(probe,root,False)
            self.assertEqual(result['status'],'INFRA_ERROR')
            self.assertEqual(result['case_report'],details)

if __name__=='__main__': unittest.main()
