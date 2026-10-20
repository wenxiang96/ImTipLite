//! ImTipLite 提示条渲染自检(不启动 GUI / 不弹窗)
//!
//! 用途: 校验"文字非空"与"视觉垂直/水平居中"两项回归, 并输出预览 BMP。
//!   cargo run --bin tip-selftest
//!
//! 退出码 0 = 全部通过, 1 = 存在失败项。
#![allow(dead_code, non_snake_case, non_upper_case_globals)]

#[path = "../tip.rs"]
mod tip;

use tip::{bmp_from_premul, build_glyphs, rasterize_tip, tip_size, Config};
use windows_sys::Win32::Graphics::GdiPlus::{GdiplusStartup, GdiplusStartupInput};

/// 在某横向区间内统计非透明像素, 返回 (left, top, right, bottom, 像素数)
fn ink_of(buf: &[u8], w: i32, h: i32, x0: i32, x1: i32) -> Option<(i32, i32, i32, i32, i32)> {
    let mut r: Option<(i32, i32, i32, i32)> = None;
    let mut n = 0;
    for y in 0..h {
        for x in x0.max(0)..x1.min(w) {
            let a = buf[((y * w + x) * 4 + 3) as usize];
            if a > 24 {
                n += 1;
                r = Some(match r {
                    None => (x, y, x, y),
                    Some((l, t, rr, b)) => (l.min(x), t.min(y), rr.max(x), b.max(y)),
                });
            }
        }
    }
    r.map(|(l, t, rr, b)| (l, t, rr, b, n))
}

fn main() {
    unsafe {
        let mut token: usize = 0;
        let mut input: GdiplusStartupInput = std::mem::zeroed();
        input.GdiplusVersion = 1;
        let rc = GdiplusStartup(&mut token, &input, std::ptr::null_mut());
        if rc != 0 {
            eprintln!("GdiplusStartup failed: {}", rc);
            std::process::exit(1);
        }

        let base = Config::default();
        let mut cfg = base.clone();
        cfg.transparent_bg = true; // 只画文字, 便于测量墨迹
        cfg.border_width = 0;
        cfg.alpha = 255;
        let scale = tip::dpi_scale_static();

        // 覆盖全部字形: 中 / 英 / 全角(。) / 半角(·) / 大写 A / 非中日韩(En)
        let cases: Vec<(&str, bool, i32, u32, i32)> = vec![
            ("中文+全角", true, 2, 0x0804, 0),
            ("中文+半角", true, 1, 0x0804, 0),
            ("中文+Caps", true, 2, 0x0804, 1),
            ("英文+半角", false, 1, 0x0804, 0),
            ("英文+全角", false, 2, 0x0804, 0),
            ("英文+Caps", false, 1, 0x0804, 1),
            ("非中文布局", false, 1, 0x0409, 0),
        ];

        let mut failures = Vec::new();
        println!("DPI scale = {:.2}", scale);
        for (tag, is_cn, symbol, lang, caps) in cases {
            let cells = build_glyphs(is_cn, symbol, lang, caps, &cfg);
            if cells.is_empty() || cells.len() > 2 {
                failures.push(format!("{}: 单元格数量 {} 不符合 1..=2", tag, cells.len()));
                continue;
            }
            let (w, h) = tip_size(&cfg, cells.len() as i32);
            let buf = match rasterize_tip(w, h, &cfg, &cells) {
                Some(b) => b,
                None => {
                    failures.push(format!("{}: rasterize_tip 返回 None", tag));
                    continue;
                }
            };
            let pad = (4.0 * scale).round() as i32;
            let cell = (cfg.cell_w as f64 * scale).round() as i32;
            print!("[{}] {}x{} cells=", tag, w, h);
            let mut pos = 0;
            for (i, (text, _)) in cells.iter().enumerate() {
                if i > 0 { print!(", "); }
                print!("{:?}", text);
                let x0 = pad + i as i32 * cell;
                let Some((l, t, r, b, n)) = ink_of(&buf, w, h, x0, x0 + cell) else {
                    failures.push(format!("{} 第 {} 格 {:?}: 没有任何可见像素(文字为空)", tag, i, text));
                    pos += 1;
                    continue;
                };
                let cx = (l + r + 1) as f64 / 2.0;
                let cy = (t + b + 1) as f64 / 2.0;
                let ex = x0 as f64 + cell as f64 / 2.0;
                let ey = h as f64 / 2.0;
                let dx = cx - ex;
                let dy = cy - ey;
                println!(
                    "\n    cell{} {:?}: ink=({},{},{},{}) px={} center=({:.1},{:.1}) expected=({:.1},{:.1}) delta=({:+.1},{:+.1})",
                    i, text, l, t, r, b, n, cx, cy, ex, ey, dx, dy
                );
                let tol = 1.0 * scale;
                if dx.abs() > tol || dy.abs() > tol {
                    failures.push(format!("{} 第 {} 格 {:?}: 未居中 delta=({:+.1},{:+.1})", tag, i, text, dx, dy));
                }
                pos += 1;
            }
            let _ = pos;
            let bmp = bmp_from_premul(w, h, &buf, 0xEC, 0xEF, 0xF4);
            let p = std::env::temp_dir().join(format!("imtip_selftest_{}.bmp", tag));
            let _ = std::fs::write(&p, &bmp);
            println!("\n    预览: {}", p.display());
        }

        if failures.is_empty() {
            println!("\nSELFTEST PASS");
        } else {
            println!("\nSELFTEST FAIL ({} 项):", failures.len());
            for f in &failures { println!("  - {}", f); }
            std::process::exit(1);
        }
    }
}
