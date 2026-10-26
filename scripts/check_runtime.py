"""Inspect native dependencies and enforce the declared release target baseline."""
import argparse
import json
import platform
from pathlib import Path
import re
import struct
import subprocess


def pe_imports(data):
    pe = struct.unpack_from('<I', data, 0x3c)[0]
    if data[pe:pe+4] != b'PE\0\0':
        raise ValueError('invalid PE signature')
    sections, optional_size = struct.unpack_from('<H', data, pe+6)[0], struct.unpack_from('<H', data, pe+20)[0]
    optional = pe+24
    magic = struct.unpack_from('<H', data, optional)[0]
    if magic not in (0x10b, 0x20b):
        raise ValueError('unsupported PE optional header')
    imports = struct.unpack_from('<I', data, optional+(112 if magic == 0x20b else 96)+8)[0]
    headers = optional+optional_size
    def offset(rva):
        for i in range(sections):
            virtual_size, address, size, raw = struct.unpack_from('<IIII', data, headers+i*40+8)
            if address <= rva < address+max(virtual_size,size):
                return raw+rva-address
        raise ValueError('unmapped PE address')
    result = []
    if imports:
        cursor = offset(imports)
        while any(data[cursor:cursor+20]):
            name = offset(struct.unpack_from('<I', data, cursor+12)[0])
            result.append(data[name:data.index(b'\0',name)].decode('ascii'))
            cursor += 20
    return sorted(result)


def inspect(binary, target):
    if 'windows' in target:
        return {'format':'PE', 'libraries':pe_imports(binary.read_bytes()),
                'validation_baseline':'Windows Server 2022', 'minimum_os_verified':False}
    if 'linux' in target:
        versions = subprocess.check_output(['readelf','--version-info',str(binary)],text=True)
        dynamic = subprocess.check_output(['readelf','-d',str(binary)],text=True)
        glibc = sorted({tuple(map(int,v.split('.'))) for v in re.findall(r'Name: GLIBC_([\d.]+)',versions)})
        if not glibc:
            raise ValueError('missing glibc dependency versions')
        if glibc[-1] > (2,35):
            raise ValueError(f'glibc dependency {glibc[-1]} exceeds release baseline 2.35')
        return {'format':'ELF','glibc_max':'.'.join(map(str,glibc[-1])),
                'libraries':re.findall(r'\(NEEDED\).*?\[(.*?)\]',dynamic),
                'validation_baseline':'Ubuntu 22.04', 'minimum_os_verified':False}
    if 'apple' in target:
        commands = subprocess.check_output(['otool','-l',str(binary)],text=True)
        libs = subprocess.check_output(['otool','-L',str(binary)],text=True)
        versions = re.findall(r'\bminos\s+(\d+(?:\.\d+)*)',commands)
        if not versions or any(tuple(map(int,v.split('.'))) > (14,0,0) for v in versions):
            raise ValueError('missing or excessive macOS deployment target')
        return {'format':'Mach-O','deployment_targets':versions,
                'libraries':[line.strip().split(' (')[0] for line in libs.splitlines()[1:]],
                'validation_baseline':'macOS 15 ARM64', 'minimum_os_verified':False}
    raise ValueError(f'unsupported runtime target: {target}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary',type=Path)
    parser.add_argument('output',type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    info = json.loads(subprocess.check_output([str(binary),'build-info','--format','json']))['data']
    report = {'build':info, 'inspection_host':platform.platform(), **inspect(binary,info['target'])}
    # GNU Windows is a developer build; the release workflow produces MSVC.
    report['release_target'] = info['target'] in ('x86_64-pc-windows-msvc','x86_64-unknown-linux-gnu','aarch64-apple-darwin')
    args.output.parent.mkdir(parents=True,exist_ok=True)
    args.output.write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    print(f'Runtime dependencies checked: {info["target"]}')


if __name__ == '__main__':
    main()
