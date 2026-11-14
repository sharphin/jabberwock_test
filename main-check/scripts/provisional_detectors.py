"""Temporary external detectors. Dedicated Jabberwock detectors are planned replacements."""
import csv
import io
import os
from pathlib import Path
import re
import shutil

TOOLS = {'inspector': ('JW_INSPECTOR', 'inspxe-cl'), 'drmemory': ('JW_DRMEMORY', 'drmemory'),
         'llvm': ('JW_OPT', 'opt'), 'ubsan': ('JW_CLANG', 'clang')}

def executable(detector):
    variable, default = TOOLS[detector]
    configured = os.environ.get(variable, default)
    found = shutil.which(configured)
    if not found and Path(configured).is_file():
        found = str(Path(configured).resolve())
    if detector == 'llvm' and not found and variable not in os.environ:
        clang = shutil.which(os.environ.get('JW_CLANG', 'clang'))
        sibling = Path(clang).with_name('opt.exe' if os.name == 'nt' else 'opt') if clang else None
        if sibling and sibling.is_file(): found = str(sibling)
    return found


def inspector_problems(text):
    """Unknown CSV schema must never be interpreted as zero problems."""
    rows = list(csv.reader(io.StringIO(text.lstrip('\ufeff'))))
    for n, row in enumerate(rows):
        names = [name.strip().casefold() for name in row]
        if 'problem type' in names:
            index = names.index('problem type')
            found = []
            for data in rows[n+1:]:
                if not data or not any(value.strip() for value in data): continue
                if len(data) != len(row) or not data[index].strip():
                    raise ValueError('unrecognized Intel Inspector CSV row')
                found.append(data[index].strip())
            return found
    raise ValueError('Intel Inspector CSV lacks Problem Type header; cannot establish coverage')


def drmemory_result(text, negative=False):
    if negative:
        return bool(re.search(r'UNINITIALIZED READ', text, re.I))
    return bool(re.search(r'\bNO ERRORS FOUND\b', text)) and not re.search(r'\bError #\d+|UNINITIALIZED READ', text, re.I)


def fixtures(detector):
    from compiler_contracts import CASES
    if detector == 'llvm':
        return [c for c in CASES if not c['reject']]
    if detector == 'inspector':
        return [c for c in CASES if 3 in c['items'] and not c['reject']]
    return [c for c in CASES if c['id'] in ('own-move-keeps-ref','array-boundary','list-write-keeps-ref','closure-own','enum-own','map-values','thread-join','channel-order','question-mark-cleanup')]


def expected_ids(detector):
    result = ['detector-negative']
    for c in fixtures(detector):
        for variant in ('elided','checked'):
            if detector == 'llvm': result.append(c['id']+':'+variant)
            else: result.extend(c['id']+'-'+variant+'-'+level for level in ('O1','O2'))
    return result
