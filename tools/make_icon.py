#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""生成 ImTipLite 应用图标(统一圆角方块 + 白色「中」)。

用法:
    python tools/make_icon.py                # 默认 teal 配色
    python tools/make_icon.py --preset blue  # 切回旧蓝色
    python tools/make_icon.py --preset violet
    python tools/make_icon.py --preset amber --glyph 英

关键点:
  * ICO 中 <=128px 的帧必须用 BMP(DIB) 编码。Windows 只对 256x256 支持 PNG 压缩帧,
    小尺寸 PNG 帧会让资源管理器/任务栏显示空白图标。
  * 所有尺寸从同一 1024px 母版高质量缩放, 保证视觉一致。
  * 母版与 tip.rs 中的托盘图标使用同一组渐变色(见 PRESETS)。
"""
import argparse
import io
import os
import struct

from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
ICON_DIR = os.path.join(ROOT, "tip-tauri", "src-tauri", "icons")

# 左上 -> 右下 对角渐变。tip.rs 的 ICON_C1 / ICON_C2 必须与此保持一致。
PRESETS = {
    "teal":   ((0x12, 0xB8, 0x86), (0x22, 0xB8, 0xCF)),  # 翡翠绿 -> 青
    "blue":   ((0x4F, 0x67, 0xD8), (0x4F, 0x67, 0xD8)),  # 旧版纯蓝
    "violet": ((0x7C, 0x5C, 0xFF), (0xE0, 0x66, 0xD6)),  # 紫 -> 品红
    "amber":  ((0xF5, 0x9F, 0x2E), (0xF2, 0x5C, 0x5C)),  # 橙 -> 珊瑚红
}

MASTER = 1024
FONT = r"C:\Windows\Fonts\msyhbd.ttc"
SIZES = [16, 20, 24, 32, 48, 64, 128, 256]
WHITE = (255, 255, 255, 255)


def rounded_mask(size, radius):
    m = Image.new("L", (size, size), 0)
    ImageDraw.Draw(m).rounded_rectangle([0, 0, size - 1, size - 1], radius=radius, fill=255)
    return m


def gradient(size, c1, c2):
    """对角线渐变(#RGB -> #RGB)。"""
    img = Image.new("RGBA", (size, size))
    px = img.load()
    span = 2 * (size - 1) or 1
    for y in range(size):
        for x in range(size):
            t = (x + y) / span
            px[x, y] = (
                round(c1[0] + (c2[0] - c1[0]) * t),
                round(c1[1] + (c2[1] - c1[1]) * t),
                round(c1[2] + (c2[2] - c1[2]) * t),
                255,
            )
    return img


def make_master(c1, c2, glyph):
    """圆角渐变方块 + 视觉居中的白色字形。"""
    img = Image.new("RGBA", (MASTER, MASTER), (0, 0, 0, 0))
    radius = int(MASTER * 0.22)
    img.paste(gradient(MASTER, c1, c2), (0, 0), rounded_mask(MASTER, radius))

    # 按墨迹包围盒居中(与提示条文字同一套"视觉居中"思想)
    font = ImageFont.truetype(FONT, int(MASTER * 0.62))
    probe = ImageDraw.Draw(Image.new("L", (MASTER, MASTER), 0))
    bbox = probe.textbbox((0, 0), glyph, font=font)
    gw, gh = bbox[2] - bbox[0], bbox[3] - bbox[1]
    x = (MASTER - gw) / 2 - bbox[0]
    y = (MASTER - gh) / 2 - bbox[1]
    ImageDraw.Draw(img).text((x, y), glyph, font=font, fill=WHITE)
    return img


def dib_frame(img):
    """BGRA 自底向上 + AND 掩码的 DIB 帧(ICO 内标准格式)。"""
    w, h = img.size
    px = img.load()
    header = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, w * h * 4, 0, 0, 0, 0)
    xor = bytearray()
    for y in range(h - 1, -1, -1):
        for x in range(w):
            r, g, b, a = px[x, y]
            xor += bytes((b, g, r, a))
    row = ((w + 31) // 32) * 4
    mask = bytearray(row * h)
    for y in range(h):
        sy = h - 1 - y
        for x in range(w):
            if px[x, sy][3] == 0:
                mask[y * row + x // 8] |= 0x80 >> (x % 8)
    return header + bytes(xor) + bytes(mask)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--preset", default="teal", choices=sorted(PRESETS))
    ap.add_argument("--glyph", default="中")
    args = ap.parse_args()

    c1, c2 = PRESETS[args.preset]
    os.makedirs(ICON_DIR, exist_ok=True)
    master = make_master(c1, c2, args.glyph)

    frames = []
    for s in SIZES:
        im = master.resize((s, s), Image.LANCZOS)
        im.save(os.path.join(ICON_DIR, f"{s}x{s}.png"))
        if s == 256:
            im.save(os.path.join(ICON_DIR, "128x128@2x.png"))
            buf = io.BytesIO()
            im.save(buf, "PNG", optimize=True)   # 256px 用 PNG 帧
            frames.append((s, buf.getvalue()))
        else:
            frames.append((s, dib_frame(im)))    # <=128px 必须用 BMP 帧
    master.resize((256, 256), Image.LANCZOS).save(os.path.join(ICON_DIR, "icon.png"))

    head = struct.pack("<HHH", 0, 1, len(frames))
    offset = 6 + 16 * len(frames)
    entries, blobs = b"", b""
    for s, data in frames:
        w = 0 if s == 256 else s
        entries += struct.pack("<BBBBHHII", w, w, 0, 0, 1, 32, len(data), offset)
        blobs += data
        offset += len(data)
    ico = head + entries + blobs
    with open(os.path.join(ICON_DIR, "icon.ico"), "wb") as f:
        f.write(ico)

    print(f"preset={args.preset} glyph={args.glyph} c1=#{c1[0]:02X}{c1[1]:02X}{c1[2]:02X} c2=#{c2[0]:02X}{c2[1]:02X}{c2[2]:02X}")
    print("icon.ico:", len(ico), "bytes;", len(frames), "frames")
    for s, d in frames:
        kind = "PNG" if d[:8] == b"\x89PNG\r\n\x1a\n" else "BMP"
        print(f"  {s}x{s}: {kind} {len(d)} bytes")


if __name__ == "__main__":
    main()
