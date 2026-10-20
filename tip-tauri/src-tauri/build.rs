fn main() {
    // tauri-build 只会为 tauri.conf.json 与 capabilities 声明 rerun-if-changed,
    // 不含图标。若不在这里补上, 用 tools/make_icon.py 重新生成 icons/icon.ico 后
    // 增量构建不会重跑 rc.exe, PE 里会残留旧图标(表现为"换了图标但没生效")。
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.png");
    println!("cargo:rerun-if-changed=icons");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=build.rs");
    tauri_build::build()
}
