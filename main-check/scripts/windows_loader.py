#!/usr/bin/env python3
"""Observe DLL load paths in one disposable diagnostic child. No registry changes."""
import argparse
import ctypes as C
from ctypes import wintypes as W
import json
import os
from pathlib import Path
import subprocess
import time


def trace(executable, timeout=20):
    report = {'modules': [], 'exceptions': [], 'debug_strings': [], 'errors': [], 'exit_code': None, 'complete': False}
    if os.name != 'nt':
        report['errors'].append('Windows debugger API required')
        return report
    if C.sizeof(C.c_void_p) != 8:
        report['errors'].append('64-bit Python required for the Windows x64 diagnostic')
        return report
    class STARTUPINFO(C.Structure):
        _fields_ = [('cb', W.DWORD), ('reserved', W.LPWSTR), ('desktop', W.LPWSTR), ('title', W.LPWSTR),
                    ('x', W.DWORD), ('y', W.DWORD), ('xs', W.DWORD), ('ys', W.DWORD),
                    ('xc', W.DWORD), ('yc', W.DWORD), ('fill', W.DWORD), ('flags', W.DWORD),
                    ('show', W.WORD), ('reserved_size', W.WORD), ('reserved_ptr', C.c_void_p),
                    ('stdin', W.HANDLE), ('stdout', W.HANDLE), ('stderr', W.HANDLE)]
    class PROCESSINFO(C.Structure):
        _fields_ = [('process', W.HANDLE), ('thread', W.HANDLE), ('pid', W.DWORD), ('tid', W.DWORD)]
    class EXCEPTIONRECORD(C.Structure):
        _fields_ = [('code', W.DWORD), ('flags', W.DWORD), ('record', C.c_void_p), ('address', C.c_void_p),
                    ('count', W.DWORD), ('info', C.c_size_t*15)]
    class EXCEPTION(C.Structure):
        _fields_ = [('record', EXCEPTIONRECORD), ('first', W.DWORD)]
    class CREATE(C.Structure):
        _fields_ = [('file', W.HANDLE), ('process', W.HANDLE), ('thread', W.HANDLE), ('base', C.c_void_p),
                    ('offset', W.DWORD), ('size', W.DWORD), ('tls', C.c_void_p), ('start', C.c_void_p),
                    ('name', C.c_void_p), ('unicode', W.WORD)]
    class LOAD(C.Structure):
        _fields_ = [('file', W.HANDLE), ('base', C.c_void_p), ('offset', W.DWORD), ('size', W.DWORD),
                    ('name', C.c_void_p), ('unicode', W.WORD)]
    class STRING(C.Structure):
        _fields_ = [('address', C.c_void_p), ('unicode', W.WORD), ('length', W.WORD)]
    class DATA(C.Union):
        _fields_ = [('exception', EXCEPTION), ('create', CREATE), ('load', LOAD), ('string', STRING),
                    ('exit', W.DWORD), ('thread', W.HANDLE)]
    class EVENT(C.Structure):
        _fields_ = [('code', W.DWORD), ('pid', W.DWORD), ('tid', W.DWORD), ('data', DATA)]
    k = C.WinDLL('kernel32', use_last_error=True)
    def api(name, args, result):
        fn = getattr(k, name); fn.argtypes = args; fn.restype = result; return fn
    create = api('CreateProcessW', [W.LPCWSTR,W.LPWSTR,C.c_void_p,C.c_void_p,W.BOOL,W.DWORD,C.c_void_p,W.LPCWSTR,C.POINTER(STARTUPINFO),C.POINTER(PROCESSINFO)], W.BOOL)
    wait = api('WaitForDebugEvent', [C.POINTER(EVENT),W.DWORD], W.BOOL)
    resume = api('ContinueDebugEvent', [W.DWORD,W.DWORD,W.DWORD], W.BOOL)
    path_of = api('GetFinalPathNameByHandleW', [W.HANDLE,W.LPWSTR,W.DWORD,W.DWORD], W.DWORD)
    close = api('CloseHandle', [W.HANDLE], W.BOOL)
    terminate = api('TerminateProcess', [W.HANDLE,W.UINT], W.BOOL)
    read = api('ReadProcessMemory', [W.HANDLE,C.c_void_p,C.c_void_p,C.c_size_t,C.POINTER(C.c_size_t)], W.BOOL)
    set_mode = api('SetErrorMode', [W.UINT], W.UINT)
    mode = set_mode(0x0001 | 0x0002 | 0x8000)
    pi = PROCESSINFO(); si = STARTUPINFO(); si.cb = C.sizeof(si)
    handles = set()
    def file_path(handle):
        if not handle:
            return None
        buf = C.create_unicode_buffer(32768)
        size = path_of(handle, buf, len(buf), 0)
        if not size or size >= len(buf):
            report['errors'].append('GetFinalPathNameByHandleW: '+str(C.get_last_error()))
            return None
        return buf.value
    try:
        command = C.create_unicode_buffer(subprocess.list2cmdline([str(executable)]))
        if not create(str(executable), command, None, None, False, 0x2 | 0x08000000, None, None, C.byref(si), C.byref(pi)):
            raise C.WinError(C.get_last_error())
        handles.update([pi.process, pi.thread])
        deadline = time.monotonic()+timeout
        while time.monotonic() < deadline:
            event = EVENT()
            if not wait(C.byref(event), 200):
                error = C.get_last_error()
                if error == 121:  # ERROR_SEM_TIMEOUT
                    continue
                raise C.WinError(error)
            continuation = 0x00010002  # DBG_CONTINUE
            if event.code == 3:  # CREATE_PROCESS_DEBUG_EVENT
                handles.update([event.data.create.process, event.data.create.thread])
                if event.data.create.file:
                    close(event.data.create.file)
            elif event.code == 2:
                handles.add(event.data.thread)
            elif event.code == 6:  # LOAD_DLL_DEBUG_EVENT
                info = event.data.load
                report['modules'].append({'path': file_path(info.file), 'base': hex(info.base or 0), 'source': 'LOAD_DLL_DEBUG_EVENT'})
                if info.file:
                    close(info.file)
            elif event.code == 1:
                exc = event.data.exception
                report['exceptions'].append({'code': hex(exc.record.code), 'first_chance': bool(exc.first), 'address': hex(exc.record.address or 0)})
                if exc.record.code != 0x80000003:
                    continuation = 0x80010001  # DBG_EXCEPTION_NOT_HANDLED: preserve child behavior
            elif event.code == 8 and len(report['debug_strings']) < 64:
                info = event.data.string
                size = min(info.length * (2 if info.unicode else 1), 65536)
                buf = C.create_string_buffer(size); received = C.c_size_t()
                if size and read(pi.process, info.address, buf, size, C.byref(received)):
                    report['debug_strings'].append(buf.raw[:received.value].decode('utf-16-le' if info.unicode else 'mbcs', errors='replace').rstrip('\0'))
            elif event.code == 5:
                report['exit_code'] = event.data.exit
                report['complete'] = True
            if not resume(event.pid, event.tid, continuation):
                raise C.WinError(C.get_last_error())
            if report['complete']:
                break
        if not report['complete']:
            report['errors'].append('loader trace timed out; diagnostic child terminated')
    except OSError as error:
        report['errors'].append(str(error))
    finally:
        if pi.process and not report['complete']:
            terminate(pi.process, 2)
        for handle in handles:
            if handle:
                close(handle)
        set_mode(mode)
    return report

if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--executable', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    a.output.write_text(json.dumps(trace(a.executable.resolve()), ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
