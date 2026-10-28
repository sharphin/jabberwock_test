#!/usr/bin/env python3
"""Verify Windows ASan and preserve actual DLL load/import/export diagnostics."""
import argparse
import ctypes
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

from windows_pe import diagnose
from windows_asan_runtime import discover, select, environment

ROOT = Path(__file__).resolve().parents[1]
LOADER_ERRORS = {0xc0000135: 'DLL_NOT_FOUND', 0xc0000139: 'ENTRYPOINT_NOT_FOUND', 0xc000007b: 'INVALID_IMAGE_FORMAT'}


def collect_diagnostics(executable, folder, env=None):
    output = folder / 'loader-trace.json'
    result = {'loader_trace': {}, 'pe_analysis': {}, 'errors': []}
    try:
        run = subprocess.run([sys.executable, str(ROOT / 'scripts/windows_loader.py'), '--executable', str(executable),
                              '--output', str(output)], capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=30, env=env)
        result['trace_command'] = {'exit': run.returncode, 'stdout': run.stdout, 'stderr': run.stderr}
        if output.is_file():
            result['loader_trace'] = json.loads(output.read_text(encoding='utf-8'))
        else:
            result['errors'].append('loader trace was not produced')
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        result['errors'].append(str(error))
    # Even when debugging is unavailable, preserve the executable's requested DLL/functions.
    try:
        result['pe_analysis'] = diagnose(executable, result['loader_trace'])
    except (OSError, ValueError) as error:
        result['errors'].append(str(error))
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / 'target/windows-asan-diagnostic.json')
    args = parser.parse_args(argv)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    selected = shutil.which(os.environ.get('JW_CLANG', 'clang'))
    report = {'schema_version': 1, 'kind': 'windows-asan-preflight', 'selected_clang': selected,
              'results': [], 'infrastructure_errors': [], 'dll_candidates': []}
    def save():
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    def record(name, passed, observed):
        entry = {'id': name, 'status': 'pass' if passed else 'fail', 'observed': observed}
        report['results'].append(entry)
        save()
        return entry
    save()
    mode = None
    try:
        if os.name != 'nt':
            raise OSError('Windows x64 preflight must run on Windows')
        # Prevent missing-DLL/error dialogs from blocking unattended subprocess capture.
        mode = ctypes.windll.kernel32.SetErrorMode(0x0001 | 0x0002 | 0x8000)
        print('Selected clang: '+str(selected), flush=True)
        discovery = discover(selected or os.environ.get('JW_CLANG', 'clang'))
        report['runtime_discovery'] = discovery
        run_env = dict(os.environ)
        verified_dll = None
        directories = ([str(Path(selected).parent)] if selected else []) + os.environ.get('PATH', '').split(os.pathsep)
        for directory in dict.fromkeys(d for d in directories if d):
            try:
                report['dll_candidates'].extend(str(p) for p in Path(directory).glob('*asan*.dll'))
            except OSError as error:
                report.setdefault('candidate_scan_errors', []).append(str(error))
        with tempfile.TemporaryDirectory(prefix='jw-asan-preflight-') as d:
            folder = Path(d)
            for name, source in (('startup', 'int main(void) { return 0; }'),
                                 ('negative', '#include <stdlib.h>\nint main(void) { volatile int *p = (int*)malloc(sizeof(int)); *p = 7; free((void*)p); return *p; }')):
                c = folder / (name+'.c'); exe = folder / (name+'.exe')
                c.write_text(source, encoding='utf-8')
                command = [selected or os.environ.get('JW_CLANG', 'clang'), '-O1', '-fsanitize=address', str(c), '-o', str(exe)]
                built = subprocess.run(command, capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=120)
                if built.returncode:
                    record(name+'-build', False, {'command': command, 'exit': built.returncode, 'stdout': built.stdout, 'stderr': built.stderr})
                    report['infrastructure_errors'].append('ASan link failed')
                    return 2
                available = discovery if verified_dll is None else {'candidates': [verified_dll]}
                selection = select(exe, available)
                report.setdefault('runtime_selections', {})[name] = selection
                if selection['selected']:
                    chosen = selection['selected']
                    if verified_dll and chosen['sha256'] != verified_dll['sha256']:
                        raise OSError('ASan runtime changed during preflight')
                    verified_dll = {**chosen, 'exists': True}
                    directory = str(Path(chosen['path']).parent)
                    run_env = environment(directory)
                    report['runtime_environment'] = {'path_prepend': directory, 'dll_path': chosen['path'], 'dll_sha256': chosen['sha256']}
                    print('Selected ASan runtime: '+chosen['path'], flush=True)
                else:
                    # Keep the original environment for the diagnostic reproduction only.
                    entry = record(name+'-runtime-selection', False, selection)
                    entry['diagnostics'] = collect_diagnostics(exe, folder, run_env)
                    report['infrastructure_errors'].append(selection['reason'])
                    print(selection['reason'], file=sys.stderr, flush=True)
                    return 2
                try:
                    run = subprocess.run([str(exe)], capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=30, env=run_env)
                    observed = {'exit': run.returncode, 'exit_hex': f'0x{run.returncode & 0xffffffff:08X}', 'stdout': run.stdout, 'stderr': run.stderr}
                    code = run.returncode & 0xffffffff
                    passed = (run.returncode == 0 if name == 'startup' else run.returncode != 0 and 'AddressSanitizer: heap-use-after-free' in run.stderr)
                except subprocess.TimeoutExpired:
                    observed = {'timeout_seconds': 30}; code = None; passed = False
                entry = record(name, passed, observed)
                if not passed:
                    entry['loader_error'] = LOADER_ERRORS.get(code, 'UNCLASSIFIED_STARTUP_OR_DETECTOR_FAILURE')
                    entry['diagnostics'] = collect_diagnostics(exe, folder, run_env)
                    report['infrastructure_errors'].append(entry['loader_error'])
                    missing = entry['diagnostics'].get('pe_analysis', {}).get('missing_exports', [])
                    for item in missing:
                        print('Missing export: '+item['function']+' in '+item['loaded_path']+' requested by '+item['requester'], flush=True)
                    if not missing:
                        print('Exact missing export unresolved; inspect loader_trace, unresolved_imports and diagnostic errors in JSON.', flush=True)
                    print('ASan preflight failed: '+entry['loader_error'], file=sys.stderr)
                    return 2
        print('ASan startup and negative control passed', flush=True)
        return 0
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        report['infrastructure_errors'].append(str(error))
        if not report['results']:
            record('preflight', False, {'error': str(error)})
        return 2
    finally:
        if mode is not None:
            ctypes.windll.kernel32.SetErrorMode(mode)
        save()
        print('ASan diagnostic report: '+str(args.output), flush=True)

if __name__ == '__main__':
    sys.exit(main())
