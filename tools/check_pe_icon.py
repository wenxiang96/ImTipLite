#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""校验 EXE 的 PE 资源中确实嵌入了图标(RT_ICON + RT_GROUP_ICON)。

背景: tauri-build 只会把 tauri.conf.json 里 bundle.icon 的 .ico 编进 PE。
      曾经出现过"源码目录图标已替换, 但 PE 里仍是旧图标/空白图标"的问题
      (ICO 内 <=128px 用 PNG 帧时 Windows 会解析失败), 所以这里做硬校验。

用法:
    python tools/check_pe_icon.py                              # 默认检查 Release 构建
    python tools/check_pe_icon.py path/to/ImTipLite.exe        # 检查指定文件

退出码: 0 通过, 1 失败
"""
import os
import struct
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT = os.path.join(ROOT, "tip-tauri", "src-tauri", "target", "release", "tip-tauri.exe")

RT_ICON = 3
RT_GROUP_ICON = 14
PNG_SIG = b"\x89PNG\r\n\x1a\n"


def parse_pe_icons(path):
    """返回 (icon_resources, group_icon_ids)。icon_resources: [(id, width, height, kind, size)]"""
    import pefile

    pe = pefile.PE(path)
    if not hasattr(pe, "DIRECTORY_ENTRY_RESOURCE"):
        return [], []

    icons, groups = [], []
    for t in pe.DIRECTORY_ENTRY_RESOURCE.entries:
        if t.id not in (RT_ICON, RT_GROUP_ICON):
            continue
        for e in t.directory.entries:
            for l in e.directory.entries:
                data = pe.get_data(l.data.struct.OffsetToData, l.data.struct.Size)
                if t.id == RT_GROUP_ICON:
                    groups.append(e.id)
                    continue
                if data[:8] == PNG_SIG:
                    w, h = struct.unpack_from(">II", data, 16)
                    icons.append((e.id, w, h, "PNG", len(data)))
                else:
                    hdr = struct.unpack_from("<IiiHH", data, 0)
                    icons.append((e.id, hdr[1], hdr[2] // 2, "BMP", len(data)))
    return icons, groups


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else DEFAULT
    if not os.path.exists(path):
        print(f"[FAIL] 文件不存在: {path}")
        return 1

    icons, groups = parse_pe_icons(path)
    ok = True

    print(f"检查: {path}")
    if not icons:
        print("  [FAIL] 没有 RT_ICON 资源(EXE 将显示空白图标)")
        ok = False
    else:
        print(f"  [ OK ] RT_ICON x{len(icons)}")
        for i, w, h, kind, size in icons:
            print(f"         id={i:<3} {w}x{h} {kind} {size} bytes")

    if not groups:
        print("  [FAIL] 没有 RT_GROUP_ICON 资源(资源管理器无法选用图标)")
        ok = False
    else:
        print(f"  [ OK ] RT_GROUP_ICON x{len(groups)} ids={groups}")

    # 关键回归点: <=128px 必须是 BMP 帧, 否则 Windows 解析失败显示空白
    bad = [x for x in icons if x[3] == "PNG" and max(x[1], x[2]) < 256]
    if bad:
        for i, w, h, _, _ in bad:
            print(f"  [FAIL] id={i} {w}x{h} 用了 PNG 帧 — <=128px 必须是 BMP 帧")
        ok = False
    else:
        print("  [ OK ] <=128px 帧均为 BMP 编码")

    print("PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
