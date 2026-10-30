#!/usr/bin/env python3
"""Win7 兼容性闸门:解析 PE 导入表,禁止 Win8+/Win10+ 才有的 API 硬导入。

背景:Rust 1.78 起 std 面向 Win10,默认产物硬导入 GetSystemTimePreciseAsFileTime
(Win8+)、api-ms-win-core-synch-l1-2-0.dll 的 WaitOnAddress 族(Win8+)、
bcryptprimitives.dll 的 ProcessPrng(Win10+),在 Win7 上加载器直接报
"无法定位程序输入点"。本项目 Windows 产物改用 x86_64-win7-windows-msvc 目标
(std 内部自动降级/延迟加载这些 API)并静态链接 CRT,本脚本验证这两点成立。

用法: python3 check_win7_imports.py <exe> [--list]
退出码: 0 通过 / 1 发现违禁导入(产物无法在 Win7 加载)
"""
import struct
import sys

# Win7 的系统 DLL 中不存在的入口点(Win8+/Win10+ 才有)
FORBIDDEN_SYMBOLS = {
    "GetSystemTimePreciseAsFileTime",                    # Win8+  kernel32
    "WaitOnAddress", "WakeByAddressAll", "WakeByAddressSingle",  # Win8+  synch API set
    "ProcessPrng",                                        # Win10+ bcryptprimitives
    "SetThreadDescription", "GetThreadDescription",       # Win10 1607+
    "GetDpiForWindow", "SetProcessDpiAwarenessContext",   # Win10 1607+/1703+
    "AdjustWindowRectExForDpi", "GetSystemMetricsForDpi",
    "SetDefaultDllDirectories",                           # Win8+  kernel32
}

# 动态 CRT 依赖(crt-static 闸门):Win7 无补丁的机器上这些 DLL 可能不存在
FORBIDDEN_DLL_EXACT = {
    "vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll",
    "ucrtbase.dll", "api-ms-win-core-synch-l1-2-0.dll",
}
FORBIDDEN_DLL_PREFIX = ("api-ms-win-crt-",)


def parse_imports(path):
    data = open(path, "rb").read()
    if data[:2] != b"MZ":
        sys.exit(f"❌ 不是 PE 文件: {path}")
    pe_off = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe_off : pe_off + 4] != b"PE\0\0":
        sys.exit("❌ 缺少 PE 签名")
    coff = pe_off + 4
    nsec = struct.unpack_from("<H", data, coff + 2)[0]
    opt_size = struct.unpack_from("<H", data, coff + 16)[0]
    opt = coff + 20
    magic = struct.unpack_from("<H", data, opt)[0]
    if magic == 0x20B:  # PE32+
        dd_off = opt + 112
        thunk_sz = 8
    elif magic == 0x10B:  # PE32
        dd_off = opt + 96
        thunk_sz = 4
    else:
        sys.exit(f"❌ 未知 Optional Header magic: {magic:#x}")
    import_rva = struct.unpack_from("<I", data, dd_off + 1 * 8)[0]

    sections = []
    sec = opt + opt_size
    for _ in range(nsec):
        vsz, va, rsz, raw = struct.unpack_from("<IIII", data, sec + 8)
        sections.append((va, vsz, raw, rsz))
        sec += 40

    def rva2off(rva):
        for va, vsz, raw, rsz in sections:
            if va <= rva < va + max(vsz, rsz):
                return raw + (rva - va)
        return None

    def cstr(off):
        end = data.index(b"\0", off)
        return data[off:end].decode("ascii", "replace")

    imports = {}  # dll_name(lower) -> set(symbol)
    if import_rva == 0:
        return imports
    desc = rva2off(import_rva)
    while True:
        oft, _ts, _fc, name_rva, ft = struct.unpack_from("<IIIII", data, desc)
        if oft == 0 and name_rva == 0 and ft == 0:
            break
        dll = cstr(rva2off(name_rva))
        thunk_rva = oft or ft
        off = rva2off(thunk_rva)
        syms = set()
        while True:
            (val,) = struct.unpack_from("<Q" if thunk_sz == 8 else "<I", data, off)
            if val == 0:
                break
            if not val & (1 << 63 if thunk_sz == 8 else 1 << 31):  # 按名导入
                syms.add(cstr(rva2off(val) + 2))  # 跳过 2 字节 Hint
            off += thunk_sz
        imports.setdefault(dll.lower(), set()).update(syms)
        desc += 20
    return imports


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    path = sys.argv[1]
    imports = parse_imports(path)

    if "--list" in sys.argv:
        for dll in sorted(imports):
            print(f"== {dll} ({len(imports[dll])}) ==")
            for s in sorted(imports[dll]):
                print("   ", s)
        return

    violations = []
    for dll, syms in sorted(imports.items()):
        if dll in FORBIDDEN_DLL_EXACT or dll.startswith(FORBIDDEN_DLL_PREFIX):
            violations.append(f"DLL {dll} (应为静态链接 CRT / 不该依赖此 API set)")
        for s in sorted(syms & FORBIDDEN_SYMBOLS):
            violations.append(f"{dll}: {s}")

    total = sum(len(s) for s in imports.values())
    print(f"导入表: {len(imports)} 个 DLL, {total} 个符号")
    if violations:
        print("❌ Win7 不兼容导入:")
        for v in violations:
            print("   -", v)
        sys.exit(1)
    print("✅ 无 Win8+/Win10+ 硬导入, 无动态 CRT 依赖 — Win7 兼容")


if __name__ == "__main__":
    main()
