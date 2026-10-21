// ImTipLite (Tauri 设置界面 + 原生提示核心)
mod tip;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{Manager, WindowEvent};

use tip::{run_tip_thread, Config, SHOW_SETTINGS};

static CFG: OnceLock<Arc<Mutex<Config>>> = OnceLock::new();
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

fn cfg_dir() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".into());
    let d = PathBuf::from(base).join("ImTipLite");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn load_config() -> Config {
    let p = cfg_dir().join("config.json");
    if let Ok(s) = std::fs::read_to_string(&p) {
        if let Ok(c) = serde_json::from_str::<Config>(&s) {
            return c;
        }
    }
    Config::default()
}

fn save_config(g: &Config) {
    let p = cfg_dir().join("config.json");
    if let Ok(s) = serde_json::to_string_pretty(g) {
        let _ = std::fs::write(&p, s);
    }
}

fn show_settings() {
    if let Some(h) = APP_HANDLE.get() {
        if let Some(w) = h.get_webview_window("main") {
            // 可能被关闭按钮隐藏 / 最小化: 先复原再显示并置前
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

fn exit_app() {
    if let Some(h) = APP_HANDLE.get() { h.exit(0); }
}

#[tauri::command]
fn get_config() -> Config {
    CFG.get().unwrap().lock().unwrap().clone()
}

#[tauri::command]
fn set_config(mut cfg: Config) -> Config {
    let c = CFG.get().unwrap();
    let mut g = c.lock().unwrap();
    // 钳制合法范围
    cfg.timeout = cfg.timeout.clamp(0.3, 10.0);
    cfg.poll_interval = cfg.poll_interval.clamp(10, 500);
    cfg.alpha = cfg.alpha.clamp(40, 255);
    cfg.font_size = cfg.font_size.clamp(9, 48);
    cfg.cell_w = cfg.cell_w.clamp(14, 80);
    cfg.height = cfg.height.clamp(16, 90);
    cfg.radius = cfg.radius.clamp(0, 45);
    cfg.border_width = cfg.border_width.clamp(0, 8);
    // 字体: 去掉首尾空白; 空则回退默认; GDI 的 LOGFONT.lfFaceName 上限 32 字符
    cfg.font_name = cfg.font_name.trim().to_string();
    if cfg.font_name.is_empty() {
        cfg.font_name = "Microsoft YaHei UI".into();
    }
    if cfg.font_name.chars().count() > 31 {
        cfg.font_name = cfg.font_name.chars().take(31).collect();
    }
    // 图标字符: 空/超长回退默认(默认值取自 Config::default)
    let d = Config::default();
    for (v, dv) in [
        (&mut cfg.char_cn, &d.char_cn),
        (&mut cfg.char_en, &d.char_en),
        (&mut cfg.char_caps, &d.char_caps),
        (&mut cfg.char_full, &d.char_full),
        (&mut cfg.char_half, &d.char_half),
        (&mut cfg.char_other, &d.char_other),
    ] {
        let t = v.trim();
        if t.is_empty() || t.chars().count() > 4 {
            *v = dv.clone();
        } else {
            *v = t.to_string();
        }
    }
    *g = cfg.clone();
    save_config(&g);
    g.clone()
}

#[tauri::command]
fn get_autostart() -> bool {
    unsafe { tip::autostart_enabled() }
}

#[tauri::command]
fn set_autostart(enabled: bool) -> bool {
    unsafe { tip::set_autostart(enabled) }
}

/// 本机已安装的字体族名(设置页字体下拉用)
#[tauri::command]
fn list_fonts() -> Vec<String> {
    unsafe { tip::list_system_fonts() }
}

#[tauri::command]
fn tip_preview(cfg: Config) -> Vec<String> {
    // 用传入的配置离屏渲染示例提示条(与真实提示条同一光栅化代码)
    tip::preview_data_urls(&cfg)
}

#[tauri::command]
fn reset_config() -> Config {
    let c = CFG.get().unwrap();
    let mut g = c.lock().unwrap();
    *g = Config::default();
    save_config(&g);
    g.clone()
}

#[tauri::command]
fn toggle_pause() -> bool {
    let c = CFG.get().unwrap();
    let mut g = c.lock().unwrap();
    g.paused = !g.paused;
    g.paused
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 静默启动: 开机自启(注册表项带 --silent 参数)时不弹设置窗口, 只在后台跑提示核心。
    // 用户手动双击 exe / 托盘打开时正常显示。
    let silent = std::env::args().nth(1).map(|a| a == "--silent").unwrap_or(false);

    // 单实例:已有一个实例在运行时,本次启动只把它的设置窗口唤到前面,然后退出。
    unsafe {
        if !tip::try_acquire_single_instance() {
            tip::signal_existing_instance();
            return;
        }
    }

    let cfg = Arc::new(Mutex::new(load_config()));
    let _ = CFG.set(cfg.clone());

    let _ = SHOW_SETTINGS.set(Box::new(show_settings));
    let _ = tip::EXIT_APP.set(Box::new(exit_app));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_config, set_config, reset_config, toggle_pause, tip_preview,
            get_autostart, set_autostart, list_fonts
        ])
        .setup({
            let cfg = cfg.clone();
            move |app| {
                let _ = APP_HANDLE.set(app.handle().clone());
                // 启动原生提示核心(工作线程)
                run_tip_thread(cfg);
                // tauri.conf.json 里 visible=false(静默启动); 手动启动时在这里显示。
                if !silent {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
                // 关闭设置窗口只是隐藏: 提示核心继续常驻, 双击托盘/exe 可重新打开
                if let Some(w) = app.get_webview_window("main") {
                    let hide_target = w.clone();
                    w.on_window_event(move |e| {
                        if let WindowEvent::CloseRequested { api, .. } = e {
                            api.prevent_close();
                            let _ = hide_target.hide();
                        }
                    });
                }
                Ok(())
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
