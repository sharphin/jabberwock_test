"""Read PE imports/exports without loading DLL code. Standard library only."""
import hashlib
from pathlib import Path
import struct

class PE:
    def __init__(self, path):
        self.path = Path(path)
        if self.path.stat().st_size > 256 * 1024 * 1024:
            raise ValueError("PE file exceeds diagnostic size limit")
        self.data = self.path.read_bytes()
        if self.data[:2] != b"MZ":
            raise ValueError("not a PE executable")
        header = self.u32(0x3c)
        if self.data[header:header+4] != b"PE\0\0":
            raise ValueError("invalid PE header")
        self.machine = self.u16(header+4)
        count, size = self.u16(header+6), self.u16(header+20)
        optional = header+24
        magic = self.u16(optional)
        if magic not in (0x10b, 0x20b):
            raise ValueError("unsupported PE optional header")
        self.width = 8 if magic == 0x20b else 4
        directory = optional + (112 if self.width == 8 else 96)
        self.directory_count = min(self.u32(directory-4), (optional+size-directory)//8)
        self.directory = directory
        self.headers_size = self.u32(optional+60)
        self.sections = []
        if count > 1024:
            raise ValueError("too many PE sections")
        for n in range(count):
            at = optional+size+40*n
            self.sections.append((self.u32(at+12), self.u32(at+16), self.u32(at+20)))

    def u16(self, at):
        return struct.unpack_from('<H', self.data, at)[0]
    def u32(self, at):
        return struct.unpack_from('<I', self.data, at)[0]
    def offset(self, rva):
        if rva < self.headers_size and rva < len(self.data):
            return rva
        for start, size, raw in self.sections:
            if start <= rva < start+size and raw+rva-start < len(self.data):
                return raw+rva-start
        raise ValueError("unmapped PE RVA: " + hex(rva))
    def string(self, rva):
        start = self.offset(rva)
        end = self.data.find(b'\0', start, min(start+65536, len(self.data)))
        if end < 0:
            raise ValueError("unterminated PE string")
        return self.data[start:end].decode('ascii', errors='replace')
    def directory_entry(self, number):
        if number >= self.directory_count:
            return 0, 0
        at = self.directory + 8*number
        return self.u32(at), self.u32(at+4)
    def imports(self):
        rva, size = self.directory_entry(1)
        if not rva:
            return []
        result = []
        for n in range(min(size//20, 4096)):
            at = self.offset(rva+20*n)
            original, _, _, name, first = struct.unpack_from('<IIIII', self.data, at)
            if not name:
                return result
            functions = []
            for i in range(65536):
                entry = struct.unpack_from('<Q' if self.width == 8 else '<I', self.data,
                                           self.offset((original or first)+i*self.width))[0]
                if not entry:
                    break
                functions.append('#'+str(entry & 0xffff) if entry & (1 << (self.width*8-1)) else self.string(entry+2))
            else:
                raise ValueError("unterminated PE import thunk")
            result.append({'dll': self.string(name), 'functions': functions})
        return result
    def exports(self):
        rva, size = self.directory_entry(0)
        if not rva:
            return {}
        at = self.offset(rva)
        base, count, names, functions, name_rvas, ordinals = struct.unpack_from('<IIIIII', self.data, at+16)
        if max(count, names) > 1000000:
            raise ValueError("export table exceeds diagnostic limit")
        result = {}
        for i in range(count):
            address = self.u32(self.offset(functions+i*4))
            if address:
                result['#'+str(base+i)] = self.string(address) if rva <= address < rva+size else None
        for i in range(names):
            name = self.string(self.u32(self.offset(name_rvas+i*4)))
            ordinal = '#'+str(base+self.u16(self.offset(ordinals+i*2)))
            if ordinal in result:
                result[name] = result[ordinal]
        return result
    def identity(self):
        return {'path': str(self.path), 'machine': hex(self.machine), 'sha256': hashlib.sha256(self.data).hexdigest()}


def diagnose(executable, trace):
    """Only observed LOAD_DLL paths count as actual resolutions; never guess from PATH."""
    loaded = {}
    result = {'modules': [], 'missing_exports': [], 'unresolved_imports': [], 'errors': [],
              'scope': 'Normal PE imports of observed modules; API-set mapping and delay imports may remain unresolved'}
    paths = [str(executable)] + [m['path'] for m in trace.get('modules', []) if m.get('path')]
    for path in dict.fromkeys(paths):
        try:
            pe = PE(path)
            key = Path(path).name.casefold()
            if key in loaded and (loaded[key] is None or str(loaded[key].path).casefold() != str(pe.path).casefold()):
                loaded[key] = None
                result['errors'].append({'path': path, 'error': 'ambiguous loaded DLL basename; resolution is not guessed'})
            else:
                loaded[key] = pe
            result['modules'].append({**pe.identity(), 'imports': pe.imports()})
        except (OSError, ValueError, struct.error) as error:
            result['errors'].append({'path': path, 'error': str(error)})
    def resolve(requester, dll, function, chain=()):
        key = (dll.casefold(), function)
        if key in chain or len(chain) > 20:
            result['unresolved_imports'].append({'requester': requester, 'dll': dll, 'function': function, 'reason': 'forwarder cycle/depth'})
            return
        target = loaded.get(dll.casefold())
        if target is None:
            result['unresolved_imports'].append({'requester': requester, 'dll': dll, 'function': function,
                'reason': 'DLL not observed under this name; may be API-set redirection or incomplete loader trace'})
            return
        try:
            if str(target.path) not in export_cache:
                export_cache[str(target.path)] = target.exports()
            exports = export_cache[str(target.path)]
            if function not in exports:
                result['missing_exports'].append({'requester': requester, 'dll': dll, 'loaded_path': str(target.path),
                                                 'function': function, 'forwarder_chain': list(chain)})
            elif exports[function]:
                library, symbol = exports[function].rsplit('.', 1)
                resolve(requester, library if library.lower().endswith('.dll') else library+'.dll', symbol, (*chain, key))
        except (ValueError, struct.error) as error:
            result['errors'].append({'path': str(target.path), 'error': str(error)})
    export_cache = {}
    for module in result['modules']:
        for item in module['imports']:
            for symbol in item['functions']:
                resolve(module['path'], item['dll'], symbol)
    return result
