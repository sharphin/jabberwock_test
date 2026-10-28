"""Select an ASan DLL from the selected clang installation, never an arbitrary PATH DLL."""
import os
from pathlib import Path
import struct
import subprocess
from windows_pe import PE

DLL = 'clang_rt.asan_dynamic-x86_64.dll'


def discover(clang):
    candidates = []
    queries = []
    def add(path, origin):
        path = Path(path)
        if path.is_absolute() and not any(x['path'] == str(path) for x in candidates):
            candidates.append({'path': str(path), 'origin': origin, 'exists': path.is_file()})
    for flag in ('--print-runtime-dir', '--print-resource-dir', '--print-file-name='+DLL):
        try:
            run = subprocess.run([clang, flag], capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=10)
            queries.append({'command': [clang, flag], 'exit': run.returncode, 'stdout': run.stdout, 'stderr': run.stderr})
            value = run.stdout.strip()
            if run.returncode or not value or '\n' in value:
                continue
            path = Path(value)
            if not path.is_absolute():
                continue  # clang echoes the basename when it cannot find the file.
            if flag.startswith('--print-file-name='):
                add(path, flag)
            elif flag == '--print-runtime-dir':
                add(path / DLL, flag)
            else:
                add(path / 'lib' / 'windows' / DLL, flag)
                add(path / 'bin' / DLL, flag)
        except (OSError, subprocess.TimeoutExpired) as error:
            queries.append({'command': [clang, flag], 'error': str(error)})
    add(Path(clang).resolve().parent / DLL, 'clang executable directory')
    return {'queries': queries, 'candidates': candidates}


def select(executable, discovery):
    image = PE(executable)
    requirements = [i for i in image.imports() if i['dll'].casefold() == DLL.casefold()]
    result = {'required_dll': DLL, 'required_functions': sorted({f for i in requirements for f in i['functions']}),
              'candidates': [], 'selected': None}
    if image.machine != 0x8664 or not requirements:
        result['reason'] = 'preflight is not an x64 executable importing the expected ASan DLL'
        return result
    for candidate in discovery['candidates']:
        inspected = dict(candidate)
        result['candidates'].append(inspected)
        if not candidate['exists']:
            continue
        try:
            dll = PE(candidate['path']); exports = dll.exports()
            inspected.update(dll.identity())
            inspected['missing_functions'] = sorted(set(result['required_functions']) - exports.keys())
            if dll.machine != image.machine:
                inspected['reason'] = 'architecture mismatch'
            elif inspected['missing_functions']:
                inspected['reason'] = 'required exports missing'
            elif result['selected'] is None:
                result['selected'] = inspected.copy()
        except (OSError, ValueError, struct.error) as error:
            inspected['error'] = str(error)
    if result['selected'] is None:
        result['reason'] = 'No compatible ASan DLL found in selected clang installation; install its matching compiler-rt runtime. A Visual Studio PATH DLL is not substituted.'
    return result


def environment(directory, base=None):
    env = dict(os.environ if base is None else base)
    env['PATH'] = str(directory) + os.pathsep + env.get('PATH', '')
    return env
