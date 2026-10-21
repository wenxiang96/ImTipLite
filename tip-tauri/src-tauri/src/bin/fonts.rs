//! 打印本机已安装字体(验证 list_system_fonts 是否正常工作)
//!   cargo run --release --bin fonts
#![allow(dead_code, non_snake_case, non_upper_case_globals)]

#[path = "../tip.rs"]
mod tip;

fn main() {
    let fonts = unsafe { tip::list_system_fonts() };
    println!("已安装字体族: {} 个", fonts.len());
    for (i, name) in fonts.iter().enumerate() {
        if i < 30 {
            println!("  {:>3}. {}", i + 1, name);
        }
    }
    if fonts.len() > 30 {
        println!("  ... 其余 {} 个省略", fonts.len() - 30);
    }
    println!();
    for probe in ["Microsoft YaHei UI", "微软雅黑", "SimHei", "Consolas", "Segoe UI"] {
        let hit = fonts.iter().any(|x| x.eq_ignore_ascii_case(probe));
        println!("  {:?} -> {}", probe, if hit { "有" } else { "无" });
    }
    // 竖排变体不应出现
    let vertical = fonts.iter().filter(|x| x.starts_with('@')).count();
    println!("\n@ 竖排变体: {} 个(应为 0)", vertical);

    // 写一份 UTF-8 清单, 便于核对中文名 / 用户级字体
    let p = std::env::temp_dir().join("imtip_fonts.txt");
    let _ = std::fs::write(&p, fonts.join("\n"));
    println!("清单已写入: {}", p.display());
}
