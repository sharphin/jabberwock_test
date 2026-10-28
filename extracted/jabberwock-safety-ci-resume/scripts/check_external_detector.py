#!/usr/bin/env python3
"""Provisional, per-compiler external detector probes; not whole-item certification."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

from compiler_contracts import CASES
from provisional_detectors import executable, inspector_problems, drmemory_result, fixtures, expected_ids

ROOT = Path(__file__).resolve().parents[1]
DRIVER = r'''
#include <stdio.h>
#include <stdlib.h>
int jw_probe_runtime_main(int argc, char **argv);
int main(int argc, char **argv) {
    const char *out = getenv("JW_PROBE_OUT"), *err = getenv("JW_PROBE_ERR"), *done = getenv("JW_PROBE_DONE");
    if (!out || !err || !done || !freopen(out, "wb", stdout) || !freopen(err, "wb", stderr)) return 120;
    int code = jw_probe_runtime_main(argc, argv);
    FILE *f = fopen(done, "wb");
    if (!f) return 121;
    fprintf(f, "%d", code);
    if (fclose(f)) return 122;
    return code;
}
'''
UB_FAULT = 'volatile int jw_zero; void jw_detector_fault(void) { volatile int x = 7 / jw_zero; (void)x; }'


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--subject', choices=('jwc','rust'), required=True)
    p.add_argument('--compiler', type=Path, required=True)
    p.add_argument('--detector', choices=('inspector','drmemory','ubsan','llvm'), required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    tool = executable(a.detector)
    report = {'subject': a.subject, 'detector': a.detector, 'provisional': True,
              'replacement': 'dedicated Jabberwock detector planned', 'results': [], 'infrastructure_errors': [],
              'expected_case_ids': expected_ids(a.detector), 'tool': tool, 'scope': 'Selected native fixtures and deliberate detector negative; no full coverage claim'}
    a.output.parent.mkdir(parents=True, exist_ok=True)
    def save(): a.output.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    def record(name, passed, observed):
        report['results'].append({'id':name,'compiler':a.subject,'status':'pass' if passed else 'fail','observed':observed})
        print(a.subject+' '+a.detector+' '+name+': '+('pass' if passed else 'fail'),flush=True); save()
    def run(command, env=None, timeout=180):
        current = dict(os.environ)
        for key in ('JW_CFLAGS','JW_NATIVE','JW_TEST_FAIL_ALLOCATION','JW_RETIREMENTS','JW_PROBE_OUT','JW_PROBE_ERR','JW_PROBE_DONE'):
            current.pop(key,None)
        current.update(env or {})
        r = subprocess.run(list(map(str,command)),cwd=ROOT,env=current,capture_output=True,text=True,encoding='utf-8',errors='replace',timeout=timeout)
        return {'command':list(map(str,command)),'exit':r.returncode,'stdout':r.stdout,'stderr':r.stderr}
    def generated(source, checked):
        options = (['--ll-checked'] if checked else ['--ll']) if a.subject=='jwc' else (['--no-elide','--ll'] if checked else ['--ll'])
        r=run([a.compiler.resolve(),*options,source])
        if r['exit'] or 'define void @jw_main(' not in r['stdout']:
            raise ValueError('selected compiler did not generate LLVM: '+json.dumps(r,ensure_ascii=False))
        return r['stdout']
    try:
        if not tool: raise ValueError('detector executable unavailable')
        report['compiler_sha256']=hashlib.sha256(a.compiler.read_bytes()).hexdigest()
        report['runtime_sha256']=hashlib.sha256((ROOT/'runtime/jw_rt.c').read_bytes()).hexdigest()
        report['tool_version']=run([tool,'-version' if a.detector in ('inspector','drmemory') else '--version'],timeout=30)
        with tempfile.TemporaryDirectory(prefix='jw-external-'+a.subject+'-') as temporary:
            folder=Path(temporary)
            seed=folder/'negative.jw'
            seed.write_text('fn work() {} fn main() { let a = spawn(work); let b = spawn(work); a.join(); b.join(); }' if a.detector=='inspector' else 'fn main() {}',encoding='utf-8')
            original=generated(seed,False)
            if a.detector=='llvm':
                bad=folder/'bad.ll'
                bad.write_text(original+'\ndefine void @jw_bad_phi() {\nentry:\n br label %done\ndone:\n %x = phi i64 [0, %entry], [1, %entry]\n ret void\n}\n',encoding='utf-8')
                check=run([tool,'-passes=verify','-disable-output',bad])
                valid=check['exit'] != 0 and bool(re.search(r'PHI|verification|Broken module',check['stderr'],re.I))
                record('detector-negative',valid,check)
                if not valid: return 1
                selected=fixtures(a.detector)
                for c in selected:
                    source=folder/(c['id']+'.jw');source.write_text(c['source'],encoding='utf-8')
                    for checked in (False,True):
                        ll=folder/(c['id']+'.ll');ll.write_text(generated(source,checked),encoding='utf-8')
                        result=run([tool,'-passes=verify','-disable-output',ll])
                        record(c['id']+(':checked' if checked else ':elided'),result['exit']==0,result)
            else:
                clang=os.environ.get('JW_CLANG','clang')
                flags=['-fsanitize=undefined','-fno-sanitize-recover=undefined'] if a.detector=='ubsan' else []
                link=['-lshell32','-Wl,/stack:268435456'] if os.name=='nt' else ['-lm','-pthread']
                driver=folder/'driver.c';driver.write_text(DRIVER,encoding='utf-8')
                fault=folder/'fault.c';fault.write_text(UB_FAULT,encoding='utf-8')
                def build(text, name, level, negative=False):
                    ll=folder/(name+'.ll');ll.write_text(text,encoding='utf-8')
                    obj=folder/(name+'.o');exe=folder/(name+('.exe' if os.name=='nt' else ''))
                    opt=['-'+level]+(['-flto','-fuse-ld=lld'] if level=='O2' else [])
                    compiled=run([clang,'-'+level,*(['-flto'] if level=='O2' else []),*flags,'-Dmain=jw_probe_runtime_main','-c',ROOT/'runtime/jw_rt.c','-o',obj])
                    if compiled['exit']: raise ValueError('runtime compilation failed: '+json.dumps(compiled))
                    compiled=run([clang,*opt,*flags,ll,obj,driver,*([fault] if negative and a.detector=='ubsan' else []),*link,'-o',exe])
                    if compiled['exit']: raise ValueError('detector link failed: '+json.dumps(compiled))
                    return exe
                def observe(exe, name, negative=False):
                    logs=folder/(name+'-logs');logs.mkdir()
                    out=logs/'target.stdout';err=logs/'target.stderr';done=logs/'target.exit'
                    env={'JW_PROBE_OUT':str(out),'JW_PROBE_ERR':str(err),'JW_PROBE_DONE':str(done),'JW_LEAKS':'1'}
                    if a.detector=='drmemory':
                        command=[tool,'-batch','-logdir',logs,'-exit_code_if_errors','86','--',exe]
                    elif a.detector=='inspector':
                        command=[tool,'-collect','ti3','-result-dir',logs/'result','--',exe]
                    else: command=[exe]
                    result=run(command,env,timeout=300)
                    result['target_stdout']=out.read_text(encoding='utf-8',errors='replace') if out.is_file() else None
                    result['target_stderr']=err.read_text(encoding='utf-8',errors='replace') if err.is_file() else None
                    result['target_exit']=done.read_text(encoding='utf-8') if done.is_file() else None
                    if a.detector=='drmemory':
                        texts=[f.read_text(encoding='utf-8',errors='replace') for f in logs.rglob('results.txt')]
                        result['detector_reports']=texts
                        detected=bool(texts) and any(drmemory_result(t,True) for t in texts)
                        clean=bool(texts) and all(drmemory_result(t) for t in texts)
                    elif a.detector=='inspector':
                        csv=run([tool,'-report','problems','-format','csv','-result-dir',logs/'result'],timeout=120)
                        result['detector_report']=csv
                        if csv['exit']: raise ValueError('Inspector report failed: '+json.dumps(csv))
                        problems=inspector_problems(csv['stdout']);result['problems']=problems
                        detected=any('data race' in x.casefold() for x in problems);clean=not problems
                    else:
                        diagnostics=(result['target_stderr'] or '') + result['stderr']
                        detected='runtime error: division by zero' in diagnostics
                        clean='runtime error:' not in diagnostics
                    result['detected_expected_fault']=detected;result['detector_clean']=clean
                    return result
                if a.detector=='inspector':
                    body='  br label %probe.loop\nprobe.loop:\n  %probe.i = phi i64 [0, %entry], [%probe.next, %probe.loop]\n  %probe.v = load volatile i64, ptr @probe.shared\n  %probe.w = add i64 %probe.v, 1\n  store volatile i64 %probe.w, ptr @probe.shared\n  %probe.next = add i64 %probe.i, 1\n  %probe.more = icmp ult i64 %probe.next, 100000\n  br i1 %probe.more, label %probe.loop, label %probe.cont\nprobe.cont:\n'
                    function='; work\n';extra='\n@probe.shared = global i64 0\n'
                elif a.detector=='drmemory':
                    body='  %probe = call ptr @malloc(i64 8)\n  %bad = load volatile i64, ptr %probe\n  call void @jw_report_checks(i64 %bad, i64 0, i64 0)\n  call void @free(ptr %probe)\n'
                    function='; main\n';extra='\ndeclare void @jw_report_checks(i64, i64, i64)\n'
                else:
                    body='  call void @jw_detector_fault()\n';function='; main\n';extra='\ndeclare void @jw_detector_fault()\n'
                start=original.index(function);entry=original.index('entry:\n',start)+len('entry:\n')
                bad=original[:entry]+body+original[entry:]+extra
                observed=observe(build(bad,'negative','O1',True),'negative',True)
                valid=observed['detected_expected_fault']
                record('detector-negative',valid,observed)
                if not valid: return 1
                selected=fixtures(a.detector)
                for c in selected:
                    source=folder/(c['id']+'.jw');source.write_text(c['source'],encoding='utf-8')
                    for checked in (False,True):
                        ll=generated(source,checked)
                        for level in ('O1','O2'):
                            name=c['id']+('-checked-' if checked else '-elided-')+level
                            result=observe(build(ll,name,level),name)
                            valid=result['detector_clean'] and result['target_exit']=='0' and result['target_stdout']==c['stdout'] and result['target_stderr']=='[live] 0\n'
                            valid=valid and result['exit']==0
                            record(name,valid,result)
    except (OSError,ValueError,subprocess.TimeoutExpired) as error:
        report['infrastructure_errors'].append(str(error))
    finally:
        report['missing_case_ids'] = sorted(set(report['expected_case_ids']) - {r['id'] for r in report['results']})
        save()
    return 2 if report['infrastructure_errors'] or not report['results'] else int(any(r['status']!='pass' for r in report['results']))

if __name__=='__main__': sys.exit(main())
