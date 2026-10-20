#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""Verify that an EXE really embeds icon resources (RT_ICON + RT_GROUP_ICON).

Why this exists: tauri-build only compiles the .ico listed in
tauri.conf.json -> bundle.icon into the PE resource section. We have hit two
bugs that produced a blank exe icon even though everything looked fine:

  1. Every frame of icon.ico was PNG-compressed. Windows only accepts PNG
     frames for the 256x256 entry; smaller PNG frames fail to parse and the
     shell falls back to a blank icon.
  2. build.rs did not declare rerun-if-changed for icons/, so replacing
     icon.ico did not re-run the resource compiler on incremental builds.

This script fails loudly on either symptom.

Usage:
    python tools/check_pe_icon.py                            # default release build
    python tools/check_pe_icon.py path/to/ImTipLite.exe      # explicit file

Exit code: 0 = pass, 1 = fail

Output is ASCII-only on purpose: GitHub Actions runs Python with a cp1252
console encoding on Windows, where printing non-ASCII raises UnicodeEncodeError.
"""
import os
import struct
import sys

# Be defensive about console encoding even for ASCII output (e.g. non-ASCII paths).
try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
except Exception:  # pragma: no cover - very old Python
    pass

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT = os.path.join(ROOT, "tip-tauri", "src-tauri", "target", "release", "tip-tauri.exe")

RT_ICON = 3
RT_GROUP_ICON = 14
PNG_SIG = b"\x89PNG\r\n\x1a\n"


def parse_pe_icons(path):
    """Return (icons, groups).

    icons:  [(id, width, height, kind, size_bytes)]
    groups: [group_resource_id]
    """
    import pefile

    pe = pefile.PE(path)
    if not hasattr(pe, "DIRECTORY_ENTRY_RESOURCE"):
        return [], []

    icons, groups = [], []
    for entry in pe.DIRECTORY_ENTRY_RESOURCE.entries:
        if entry.id not in (RT_ICON, RT_GROUP_ICON):
            continue
        for res in entry.directory.entries:
            for lang in res.directory.entries:
                data = pe.get_data(lang.data.struct.OffsetToData, lang.data.struct.Size)
                if entry.id == RT_GROUP_ICON:
                    groups.append(res.id)
                    continue
                if data[:8] == PNG_SIG:
                    w, h = struct.unpack_from(">II", data, 16)
                    icons.append((res.id, w, h, "PNG", len(data)))
                else:
                    hdr = struct.unpack_from("<IiiHH", data, 0)
                    icons.append((res.id, hdr[1], hdr[2] // 2, "BMP", len(data)))
    return icons, groups


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else DEFAULT
    if not os.path.exists(path):
        print("[FAIL] file not found: {}".format(path))
        return 1

    icons, groups = parse_pe_icons(path)
    ok = True

    print("check: {}".format(path))
    if not icons:
        print("  [FAIL] no RT_ICON resource (shell would show a blank icon)")
        ok = False
    else:
        print("  [ OK ] RT_ICON x{}".format(len(icons)))
        for i, w, h, kind, size in icons:
            print("         id={:<3} {}x{} {} {} bytes".format(i, w, h, kind, size))

    if not groups:
        print("  [FAIL] no RT_GROUP_ICON resource (Explorer cannot select an icon)")
        ok = False
    else:
        print("  [ OK ] RT_GROUP_ICON x{} ids={}".format(len(groups), groups))

    # Regression guard: frames <=128px must be BMP encoded.
    bad = [x for x in icons if x[3] == "PNG" and max(x[1], x[2]) < 256]
    if bad:
        for i, w, h, _, _ in bad:
            print("  [FAIL] id={} {}x{} uses a PNG frame - <=128px must be BMP".format(i, w, h))
        ok = False
    elif icons:
        print("  [ OK ] all frames <=128px are BMP encoded")

    print("PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
