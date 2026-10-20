// ImTipLite 原生核心(Tauri 集成版)
// 在工作线程跑消息循环 + 原生 layered 提示窗 + 托盘。
// 配置由共享 Arc<Mutex<Config>> 提供,Tauri 命令可实时改写。

#![allow(non_snake_case, non_upper_case_globals)]

use std::cell::Cell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Foundation::{
    GetLastError, BOOL, HINSTANCE, HMODULE, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::{
    GdipCreateBitmapFromScan0, GdipCreateFontFromDC, GdipCreatePen1, GdipCreateSolidFill,
    GdipDeleteBrush, GdipDeleteFont, GdipDeleteGraphics, GdipDeletePen, GdipDeleteStringFormat,
    GdipDisposeImage, GdipDrawImageRectI, GdipDrawPolygonI, GdipDrawString, GdipMeasureString,
    GdipFillPolygonI, GdipGetImageGraphicsContext, GdipGraphicsClear,
    GdipSetCompositingMode, GdipSetInterpolationMode, GdipSetPixelOffsetMode,
    GdipSetSmoothingMode, GdipSetStringFormatAlign, GdipSetStringFormatFlags,
    GdipSetStringFormatLineAlign,
    GdipSetTextRenderingHint, GdipStringFormatGetGenericDefault, GdiplusStartup,
    GdiplusStartupInput, GpBitmap, GpFont, GpGraphics, GpImage, GpPen, GpSolidFill,
    GpStringFormat, Point, RectF,
};
use windows_sys::Win32::Foundation::SIZE;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NOTIFYICONDATAW, NIM_ADD, NIM_DELETE, NIF_ICON, NIF_MESSAGE, NIF_TIP,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetKeyboardLayout, VK_CAPITAL,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};

#[link(name = "imm32")]
extern "system" {
    fn ImmGetDefaultIMEWnd(hwnd: isize) -> isize;
}
#[link(name = "kernel32")]
extern "system" {
    fn CreateMutexW(
        lpmutexattributes: *const c_void,
        initialowner: BOOL,
        lpname: *const u16,
    ) -> isize;
}

// ---- 常量 ----
const WM_TRAY: u32 = WM_APP + 1;
/// 第二个实例向已运行实例发送: 显示设置窗口
pub const WM_SHOW_SETTINGS: u32 = WM_APP + 2;
const TIMER_POLL: usize = 1;
const WM_IME_CONTROL: u32 = 0x0283;
const IMC_GETOPENSTATUS: WPARAM = 0x0005;
const IMC_GETCONVERSIONMODE: WPARAM = 0x0001;
const SMTO_ABORTIFHUNG: u32 = 0x0002;
const ERROR_ALREADY_EXISTS: u32 = 183;
const CS_HREDRAW_VREDRAW: u32 = 0x0003;
const IDM_PAUSE: usize = 1001;
const IDM_MOUSE: usize = 1002;
const IDM_FULLSCREEN: usize = 1003;
const IDM_SHOW: usize = 1004;
const IDM_EXIT: usize = 1005;

/// 由 Tauri 主线程设置:显示设置窗口的回调。
pub static SHOW_SETTINGS: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> =
    std::sync::OnceLock::new();
/// 由 Tauri 主线程设置:完整退出应用的回调。
pub static EXIT_APP: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> =
    std::sync::OnceLock::new();

/// 单实例:尝试获取全局互斥体。返回 true 表示本进程是第一个实例。
pub unsafe fn try_acquire_single_instance() -> bool {
    let name = wstr("ImTipLite_SingleInstance");
    let m = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
    if m == 0 {
        return true; // 拿不到互斥体也放行, 不阻碍使用
    }
    GetLastError() != ERROR_ALREADY_EXISTS
}

/// 通知已运行的实例把设置窗口显示到前面。
pub unsafe fn signal_existing_instance() {
    let cls = wstr("ImTipLiteMsg");
    let title = wstr("ImTipLite");
    let hwnd = FindWindowW(cls.as_ptr(), title.as_ptr());
    if hwnd != 0 {
        PostMessageW(hwnd, WM_SHOW_SETTINGS, 0, 0);
    }
}

const BG: (u8, u8, u8) = (0x26, 0x2B, 0x33);
const COLOR_CN: (u8, u8, u8) = (0x6B, 0xE5, 0x85);
const COLOR_EN: (u8, u8, u8) = (0x6B, 0xB6, 0xFF);
const COLOR_SYMBOL: (u8, u8, u8) = (0xFF, 0xB4, 0x54);
const COLOR_CAPS: (u8, u8, u8) = (0xFF, 0xD6, 0x6B);
const COLOR_HALF: (u8, u8, u8) = (0x7A, 0x82, 0x91);
const CJK_LANG_IDS: &[u32] = &[0x0804, 0x0404, 0x0C04, 0x1404, 0x0412, 0x0411];
/// 托盘图标渐变色(RGB, 左上 -> 右下)。改图标时与 tools/make_icon.py 的 PRESETS 保持同步。
const ICON_C1: (u8, u8, u8) = (0x12, 0xB8, 0x86); // #12B886 翡翠绿
const ICON_C2: (u8, u8, u8) = (0x22, 0xB8, 0xCF); // #22B8CF 青
// GDI+ 排版格式标志: 不折行 / 不裁剪(单格文字必须完整绘制)
const STRING_FORMAT_NOWRAP_NOCLIP: i32 = 0x0000_1000 | 0x0000_4000;
// 墨迹判定阈值(4x 超采样空间): 滤掉几乎不可见的抗锯齿边缘
const INK_ALPHA_THRESHOLD: u8 = 16;

fn render_config_changed(old: Option<&Config>, new: &Config) -> bool {
    let Some(o) = old else { return true; };
    o.transparent_bg != new.transparent_bg || o.alpha != new.alpha ||
    o.radius != new.radius || o.border_width != new.border_width ||
    o.color_bg != new.color_bg || o.color_border != new.color_border ||
    o.color_cn != new.color_cn || o.color_en != new.color_en ||
    o.color_full != new.color_full || o.color_half != new.color_half ||
    o.color_caps != new.color_caps || o.font_size != new.font_size ||
    o.cell_w != new.cell_w || o.height != new.height ||
    o.font_name != new.font_name || o.offset_x != new.offset_x || o.offset_y != new.offset_y ||
    o.char_cn != new.char_cn || o.char_en != new.char_en || o.char_full != new.char_full ||
    o.char_half != new.char_half || o.char_caps != new.char_caps || o.char_other != new.char_other
}

fn wstr(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
fn now() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ---- 配置(前端可读写) ----
#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Config {
    pub paused: bool,
    pub follow_mouse: bool,
    pub ignore_fullscreen: bool,
    pub show_on_change: bool,
    pub timeout: f64,
    pub poll_interval: u32,
    pub offset_x: f64,
    pub offset_y: f64,
    pub alpha: u8,
    pub transparent_bg: bool, // 透明背景(只显示图标)
    // 外观自定义
    pub font_size: i32,     // 主字号 px(96dpi)
    pub cell_w: i32,        // 单元格宽(96dpi)
    pub height: i32,        // 窗口高(96dpi)
    pub radius: i32,        // 圆角
    pub border_width: i32,  // 边框宽(0=无边框)
    pub font_name: String,
    pub color_bg: String,
    pub color_border: String,
    pub color_cn: String,
    pub color_en: String,
    pub color_full: String,
    pub color_caps: String,
    pub color_half: String,
    // 图标字符自定义(ImTip tipChars 同款)
    pub char_cn: String,      // 中文图标
    pub char_en: String,      // 英文图标
    pub char_caps: String,    // 大写图标
    pub char_full: String,    // 全角图标
    pub char_half: String,    // 半角图标
    pub char_other: String,  // 非中日韩布局图标
}
impl Default for Config {
    fn default() -> Self {
        Config {
            paused: false,
            follow_mouse: false,
            ignore_fullscreen: true,
            show_on_change: true,
            timeout: 3.0,
            poll_interval: 50,
            offset_x: 14.0,
            offset_y: 2.0,
            alpha: 235,
            transparent_bg: false,
            font_size: 17,
            cell_w: 26,
            height: 30,
            radius: 9,
            border_width: 0,
            font_name: "Microsoft YaHei UI".into(),
            color_bg: "#262B33".into(),
            color_border: "#DBA65A".into(),
            color_cn: "#6BE585".into(),
            color_en: "#6BB6FF".into(),
            color_full: "#FFB454".into(),
            color_caps: "#FFD66B".into(),
            color_half: "#7A8291".into(),
            char_cn: "中".into(),
            char_en: "英".into(),
            char_caps: "A".into(),
            char_full: "。".into(),
            char_half: "·".into(),
            char_other: "En".into(),
        }
    }
}

// PixelFormat32bppARGB(Windows SDK: 10 | 32<<8 | Alpha | GDI | Canonical)
// 渲染用 ARGB, 上屏前手动预乘(避免 PARGB scan0 的兼容问题)
const PIXEL_FORMAT_32BPP_ARGB: i32 = 0x26200A;

fn dbg(s: &str) {
    // 仅在设置了 IMTIP_DEBUG 环境变量时才写日志, 避免平时产生日志文件
    if std::env::var_os("IMTIP_DEBUG").is_none() { return; }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true)
        .open(std::env::temp_dir().join("imtip_dbg.log")) {
        let _ = writeln!(f, "{} {}", now(), s);
    }
}

/// 诊断: 把提示条位图(预乘 BGRA)合成到白底, 写成 24bpp BMP, 便于检视边缘抗锯齿。
unsafe fn dump_bmp(w: i32, h: i32, px: *const u8) {
    dump_bmp_name(w, h, px, "imtip_dump.bmp");
}

unsafe fn dump_bmp_name(w: i32, h: i32, px: *const u8, name: &str) {
    if w <= 0 || h <= 0 { return; }
    let row = ((w * 3 + 3) / 4) * 4;
    let size = 54 + row * h;
    let mut out = vec![0u8; size as usize];
    out[0] = b'B'; out[1] = b'M';
    out[2..6].copy_from_slice(&(size as u32).to_le_bytes());
    out[10..14].copy_from_slice(&54u32.to_le_bytes());
    out[14..18].copy_from_slice(&40u32.to_le_bytes());
    out[18..22].copy_from_slice(&(w as u32).to_le_bytes());
    out[22..26].copy_from_slice(&(-h).to_le_bytes());
    out[26..28].copy_from_slice(&1u16.to_le_bytes());
    out[28..30].copy_from_slice(&24u16.to_le_bytes());
    for y in 0..h {
        for x in 0..w {
            let o = ((y * w + x) * 4) as isize;
            let b = *px.offset(o) as u32;
            let g = *px.offset(o + 1) as u32;
            let r = *px.offset(o + 2) as u32;
            let a = *px.offset(o + 3) as u32;
            let inv = 255 - a; // 预乘 + 白底补偿
            let off = (54 + y * row + x * 3) as usize;
            out[off] = (b + inv).min(255) as u8;
            out[off + 1] = (g + inv).min(255) as u8;
            out[off + 2] = (r + inv).min(255) as u8;
        }
    }
    let p = std::env::temp_dir().join(name);
    let _ = std::fs::write(&p, &out);
    dbg(&format!("dumped {}x{} -> {:?}", w, h, p));
}

fn parse_color(s: &str, fb: (u8, u8, u8)) -> (u8, u8, u8) {
    let t = s.trim_start_matches('#');
    if t.len() == 6 {
        if let Ok(v) = u32::from_str_radix(t, 16) {
            return (((v >> 16) & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8);
        }
    }
    fb
}

// ---- 检测 ----
unsafe fn ime_control(hwnd: isize, cmd: WPARAM) -> Option<usize> {
    let ime = ImmGetDefaultIMEWnd(hwnd);
    if ime == 0 {
        return None;
    }
    let mut res: usize = 0;
    let ok = SendMessageTimeoutW(ime, WM_IME_CONTROL, cmd, 0, SMTO_ABORTIFHUNG, 60, &mut res as *mut usize);
    if ok == 0 { None } else { Some(res) }
}
unsafe fn lang_id_of(hwnd: isize) -> u32 {
    let tid = GetWindowThreadProcessId(hwnd, std::ptr::null_mut());
    (GetKeyboardLayout(tid) as u32) & 0xFFFF
}
unsafe fn detect_state(focus: isize) -> Option<(bool, i32, u32, u32)> {
    let opened = ime_control(focus, IMC_GETOPENSTATUS);
    let conv = ime_control(focus, IMC_GETCONVERSIONMODE);
    if opened.is_none() && conv.is_none() { return None; }
    let lang = lang_id_of(focus);
    let mut opened = opened.unwrap_or(0) != 0;
    if opened && lang == 0x0409 { opened = false; }
    let conv = match conv { Some(c) => c as u32, None => return Some((false, 1, lang, 0)) };
    if (conv & 0x0100) != 0 { opened = false; } // IMC_GETCONVERSIONMODE 的 NOCONVERSION
    let is_cn = opened && (conv & 0x0003) != 0;
    // 全半角:
    //   · 优先采信 IME 上报的 FULLSHAPE(0x8) —— 部分第三方输入法会正确上报
    //   · 微软拼音等不上报该位(实测 conv 只在 0x0/0x1 间变化), 此时按
    //     "中文=全角、英文=半角" 的通用语义推断
    let mut symbol: i32 = if (conv & 0x0008) != 0 {
        2 // 明确上报全角
    } else if is_cn {
        2 // 中文模式视为全角
    } else {
        1 // 英文模式视为半角
    };
    if (conv & 0x400) != 0 { symbol = 3; } // 中文标点(同为全角字形)
    // 诊断: 状态变化时记录原始值
    {
        static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);
        let sig = ((opened as u64) << 40) | ((is_cn as u64) << 39) | ((symbol as u64) << 32) | (lang as u64 & 0xFFFF) | ((conv as u64 & 0xFFFF) << 48);
        if LAST.swap(sig, Ordering::Relaxed) != sig {
            dbg(&format!("state: focus=0x{:x} opened={} conv=0x{:x} lang=0x{:x} -> is_cn={} symbol={}",
                focus as usize, opened, conv, lang, is_cn, symbol));
        }
    }
    Some((is_cn, symbol, lang, conv))
}
unsafe fn caret_screen(gti: &GUITHREADINFO) -> Option<(i32, i32, i32)> {
    if gti.hwndCaret == 0 { return None; }
    // 不要求 GUI_CARETBLINKING 位: Notepad++/Scintilla 等自绘光标不置位, 但 rcCaret 有效
    let r = gti.rcCaret;
    if r.right <= r.left && r.bottom <= r.top { return None; }
    let mut bottom = POINT { x: r.left, y: r.bottom };
    if ClientToScreen(gti.hwndCaret, &mut bottom) == 0 { return None; }
    let mut top = POINT { x: r.left, y: r.top };
    ClientToScreen(gti.hwndCaret, &mut top);
    Some((bottom.x, bottom.y, top.y))
}
/// 提示条最多两格(总共最多两格):
///   第 1 格 = 中文 / 英文(中文模式显示“中”, 英文模式显示“英”)
///   第 2 格 = 全角 / 半角; CapsLock 开启时改为大写 A
/// 中文模式统一按全角、英文模式统一按半角处理(detect_state 已归一化 symbol)。
pub fn build_glyphs(is_cn: bool, symbol: i32, lang: u32, caps: i32, cfg: &Config) -> Vec<(String, (u8, u8, u8))> {
    let c_cn = parse_color(&cfg.color_cn, COLOR_CN);
    let c_en = parse_color(&cfg.color_en, COLOR_EN);
    let c_full = parse_color(&cfg.color_full, COLOR_SYMBOL);
    let c_half = parse_color(&cfg.color_half, COLOR_HALF);
    let c_caps = parse_color(&cfg.color_caps, COLOR_CAPS);
    let mut cells = Vec::with_capacity(2);
    // 第 1 格: 中 / 英(非中日韩键盘布局显示 char_other)
    let (t1, c1) = if is_cn {
        (cfg.char_cn.clone(), c_cn)
    } else if lang == 0x0411 {
        ("あ".to_string(), c_en)
    } else if lang == 0x0412 {
        ("가".to_string(), c_en)
    } else if CJK_LANG_IDS.contains(&lang) {
        (cfg.char_en.clone(), c_en)
    } else {
        (cfg.char_other.clone(), c_en)
    };
    if !t1.is_empty() { cells.push((t1, c1)); }
    // 第 2 格: CapsLock 优先显示 A, 否则全角 / 半角
    let (t2, c2) = if caps != 0 {
        let tc = if cfg.char_caps.is_empty() { "A".to_string() } else { cfg.char_caps.clone() };
        (tc, c_caps)
    } else if symbol == 1 {
        (cfg.char_half.clone(), c_half)
    } else {
        (cfg.char_full.clone(), c_full)
    };
    if !t2.is_empty() { cells.push((t2, c2)); }
    cells.truncate(2);
    cells
}

// ---- 应用 ----
struct App {
    msg_hwnd: isize,
    tip_hwnd: isize,
    visible: bool,
    show_until: f64,
    last_key: (isize, bool, i32, u32, i32),
    last_caret: (i32, i32),
    last_ime_state: Option<(bool, i32, u32, u32)>,
    font: isize,
    font_px: i32,
    font_name: String,
    draw_cells: Vec<(String, (u8, u8, u8))>,
    draw_w: i32,
    draw_h: i32,
    // 当前窗口内容是否已成功绘制(失败后下一轮必须重绘)
    painted: bool,
    mem_dc: isize,
    mem_bmp: isize,
    mem_w: i32,
    mem_h: i32,
    dib_px: *mut c_void,
    hinst: isize,
    tray_added: bool,
    tray_icon: isize,
    timer_interval: u32,
    cfg: Arc<Mutex<Config>>,
    // 上次实际应用的设置快照: 用 PartialEq 精确检测外观/位置变化
    last_render_cfg: Option<Config>,
    // 按键触发的全局悬停窗口: 触发点 + 到期时间(期间无光标窗口也保持显示)
    force_until: f64,
    force_pos: (i32, i32),
    // 等待 IME 状态稳定的截止时间(Shift 抬起后 IME 可能异步更新)
    pending_until: f64,
    // 光标开始不可用的时间(0=可用), 用于宽限避免闪烁
    no_caret_since: f64,
    // 状态去抖: 需连续 2 次一致才确认, 过滤部分程序 IME 状态抖动
    pending_key: Option<(isize, bool, i32, u32, i32)>,
    pending_count: u32,
}
impl App {
    fn new(hinst: isize, cfg: Arc<Mutex<Config>>) -> Self {
        App {
            msg_hwnd: 0, tip_hwnd: 0, visible: false, show_until: 0.0,
            last_key: (0, false, 0, 0, 0), last_caret: (i32::MIN, i32::MIN),
            last_ime_state: None,
            font: 0, font_px: 0, font_name: String::new(),
            draw_cells: Vec::new(), draw_w: 0, draw_h: 0, painted: false,
            mem_dc: 0, mem_bmp: 0, mem_w: 0, mem_h: 0,
            dib_px: std::ptr::null_mut(),
            hinst, tray_added: false, tray_icon: 0, timer_interval: 0, cfg,
            last_render_cfg: None,
            force_until: 0.0, force_pos: (0, 0), pending_until: 0.0, no_caret_since: 0.0,
            pending_key: None, pending_count: 0,
        }
    }
    fn snap(&self) -> Config { self.cfg.lock().unwrap().clone() }

    /// CapsLock 状态(钩子维护, 读取零开销)
    unsafe fn caps_state(&mut self) -> i32 {
        if CAPS_ON.load(Ordering::Relaxed) { 1 } else { 0 }
    }

    unsafe fn hide_tip(&mut self) {
        if self.visible {
            dbg("hide");
            if self.tip_hwnd != 0 { ShowWindow(self.tip_hwnd, SW_HIDE); }
        }
        self.visible = false;
    }
    unsafe fn ensure_tip(&mut self, _alpha: u8) {
        if self.tip_hwnd != 0 { return; }
        let cls = wstr("ImTipLiteTip");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, style: CS_HREDRAW_VREDRAW,
            lpfnWndProc: Some(tip_wndproc), cbClsExtra: 0, cbWndExtra: 0,
            hInstance: self.hinst as HINSTANCE, hIcon: 0, hCursor: LoadCursorW(0, IDC_ARROW),
            hbrBackground: 0, lpszMenuName: std::ptr::null(), lpszClassName: cls.as_ptr(), hIconSm: 0,
        };
        RegisterClassExW(&wc);
        let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        self.tip_hwnd = CreateWindowExW(ex, cls.as_ptr(), std::ptr::null(), WS_POPUP, 0,0,10,10, 0,0, self.hinst as HINSTANCE, std::ptr::null());
        // 逐像素 alpha 模式由 UpdateLayeredWindow 驱动, 不调用 SetLayeredWindowAttributes
    }
    unsafe fn dpi_scale(&self) -> f64 {
        let hdc = GetDC(0);
        if hdc != 0 { let dpi = GetDeviceCaps(hdc, LOGPIXELSX as i32); ReleaseDC(0, hdc); if dpi > 0 { return dpi as f64 / 96.0; } }
        1.0
    }
    unsafe fn show_tip(&mut self, caret: (i32,i32,i32), cells: Vec<(String,(u8,u8,u8))>, cfg: &Config) {
        self.ensure_tip(cfg.alpha);
        if self.tip_hwnd == 0 { return; }
        if cells.is_empty() {
            // 没有任何字形可画: 不要把空内容显示成一个小方块
            dbg("show_tip with empty cells; hide");
            self.hide_tip();
            return;
        }
        let scale = self.dpi_scale();
        let pad = (4.0*scale).round() as i32;
        let cell = (cfg.cell_w as f64 * scale).round() as i32;
        let h = (cfg.height as f64 * scale).round() as i32;
        let n = cells.len().max(1) as i32;
        let w = pad*2 + cell*n;
        let font_px = (cfg.font_size as f64 * scale).round() as i32;
        if font_px != self.font_px || self.font == 0 || self.font_name != cfg.font_name {
            if self.font != 0 { DeleteObject(self.font); }
            let fname = wstr(&cfg.font_name);
            self.font = CreateFontW(-font_px,0,0,0,FW_BOLD as i32,0,0,0,DEFAULT_CHARSET as u32,0,0,0,0, fname.as_ptr());
            self.font_px = font_px;
            self.font_name = cfg.font_name.clone();
        }
        let (cx, cy_bottom, cy_top) = caret;
        let mut x = cx + (cfg.offset_x*scale).round() as i32;
        let mut y = cy_bottom + (cfg.offset_y*scale).round() as i32;
        let mut mi: MONITORINFO = unsafe { std::mem::zeroed() };
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let mon = MonitorFromPoint(POINT { x: cx, y: cy_bottom }, MONITOR_DEFAULTTONEAREST as u32);
        if mon != 0 && GetMonitorInfoW(mon, &mut mi) != 0 {
            let work = mi.rcWork;
            if y + h > work.bottom { y = cy_top - h - (cfg.offset_y*scale).round() as i32; }
            if x + w > work.right { x = work.right - w - 4; }
            if x < work.left { x = work.left; }
            if y < work.top { y = work.top; }
        }
        let size_changed = w != self.draw_w || h != self.draw_h;
        let content_changed = cells != self.draw_cells;
        let cfg_changed = render_config_changed(self.last_render_cfg.as_ref(), cfg);
        // 关键: 先把本轮最新内容写入 self.draw_cells, render() 只从这里取文字/颜色。
        // 否则会出现窗口已弹出但内容仍是上一轮(或首次为空)的问题。
        self.draw_cells = cells;
        self.draw_w = w;
        self.draw_h = h;
        dbg(&format!("show_tip cells={:?} {}x{} size_changed={} content_changed={} cfg_changed={} painted={}",
            self.draw_cells.iter().map(|c| c.0.clone()).collect::<Vec<_>>(), w, h,
            size_changed, content_changed, cfg_changed, self.painted));
        SetWindowPos(self.tip_hwnd, -1, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        if size_changed || content_changed || cfg_changed || !self.painted {
            if !self.render(w, h, cfg) {
                dbg("render failed; hide hint window");
                self.painted = false;
                self.hide_tip();
                return;
            }
            self.painted = true;
            self.last_render_cfg = Some(cfg.clone());
        }
        self.visible = true;
    }

    /// GDI+ 渲染到 32bpp DIB + UpdateLayeredWindow 逐像素 alpha(圆角/文字抗锯齿)
    unsafe fn render(&mut self, w: i32, h: i32, cfg: &Config) -> bool {
        if self.tip_hwnd == 0 || w <= 0 || h <= 0 { return false; }
        // 确保目标 DIB(供 UpdateLayeredWindow 读取)
        if self.mem_dc == 0 || self.mem_w != w || self.mem_h != h {
            if self.mem_bmp != 0 { DeleteObject(self.mem_bmp as HGDIOBJ); self.mem_bmp = 0; }
            if self.mem_dc != 0 { DeleteDC(self.mem_dc); self.mem_dc = 0; }
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w;
            bmi.bmiHeader.biHeight = -h; // 自顶向下
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            let mut px: *mut c_void = std::ptr::null_mut();
            let screen = GetDC(0);
            self.mem_bmp = CreateDIBSection(screen, &bmi, 0, &mut px, 0, 0);
            self.mem_dc = CreateCompatibleDC(screen);
            ReleaseDC(0, screen);
            if self.mem_bmp == 0 || self.mem_dc == 0 { return false; }
            // DIB 不长期选入 DC: GDI+ 不允许在选入 DC 的位图上渲染(ObjectBusy)
            self.dib_px = px;
            self.mem_w = w; self.mem_h = h;
        }
        if self.dib_px.is_null() { return false; }
        // 与设置页预览共用同一光栅化函数, 确保两者完全一致
        let cells = self.draw_cells.clone();
        dbg(&format!("render cells={:?} {}x{}", cells.iter().map(|c| c.0.clone()).collect::<Vec<_>>(), w, h));
        if let Some(buf) = rasterize_tip(w, h, cfg, &cells) {
            let n = (w * h * 4) as usize;
            std::ptr::copy_nonoverlapping(buf.as_ptr(), self.dib_px as *mut u8, n);
        } else {
            dbg(&format!("rasterize_tip failed {}x{}; skip ULW", w, h));
            return false;
        }
        if std::env::var_os("IMTIP_DUMP").is_some() {
            dump_bmp(w, h, self.dib_px as *const u8);
        }
        // 上屏: 临时把 DIB 选入 DC 供 ULW 读取, 用完立即取出
        let md = self.mem_dc;
        let oldsel = SelectObject(md, self.mem_bmp as HGDIOBJ);
        let sz = SIZE { cx: w, cy: h };
        let ptsrc = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION { BlendOp: 0 /*AC_SRC_OVER*/, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: 1 /*AC_SRC_ALPHA*/ };
        let ok = UpdateLayeredWindow(self.tip_hwnd, 0, std::ptr::null(), &sz, md, &ptsrc, 0, &blend, ULW_ALPHA);
        SelectObject(md, oldsel);
        if ok == 0 {
            dbg(&format!("ULW failed lasterr={}", GetLastError()));
            false
        } else {
            dbg(&format!("ULW ok {}x{}", w, h));
            true
        }
    }
    unsafe fn is_fullscreen(&self, fg: isize) -> bool {
        let style = GetWindowLongPtrW(fg, GWL_STYLE);
        if (style as u32) & WS_CAPTION != 0 { return false; }
        let mut wr: RECT = unsafe { std::mem::zeroed() };
        if GetWindowRect(fg, &mut wr) == 0 { return false; }
        let mut mi: MONITORINFO = unsafe { std::mem::zeroed() };
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST as u32);
        if mon == 0 || GetMonitorInfoW(mon, &mut mi) == 0 { return false; }
        let m = mi.rcMonitor;
        wr.left <= m.left+2 && wr.top <= m.top+2 && wr.right >= m.right-2 && wr.bottom >= m.bottom-2
    }
    unsafe fn poll(&mut self) {
        let cfg = self.snap();
        // 轮询间隔变化时重设定时器
        if cfg.poll_interval != self.timer_interval {
            self.timer_interval = cfg.poll_interval;
            KillTimer(self.msg_hwnd, TIMER_POLL);
            SetTimer(self.msg_hwnd, TIMER_POLL, cfg.poll_interval.max(10), None);
        }
        // 哨兵值: 无有效输入目标时重置, 使下次出现光标必然触发显示
        let sentinel: (isize, bool, i32, u32, i32) = (0, false, 0, 0, 0);
        // force 提前消费: 无论后续哪个环节提前返回, 都不会产生延迟弹窗
        let force = SHOW_HINT.swap(false, Ordering::Relaxed);
        if force { dbg("poll: force consumed"); }
        let t = now();
        if force {
            // 记下悬停窗口与触发点
            self.force_until = t + cfg.timeout;
            // 等待 IME 状态稳定的截止时间(Shift 抬起后可能异步更新)
            self.pending_until = t + 0.3;
            let mut pt = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut pt) != 0 { self.force_pos = (pt.x, pt.y); }
        }
        let in_force = t < self.force_until;
        if cfg.paused { self.hide_tip(); self.last_key = sentinel; self.force_until = 0.0; return; }
        let fg = GetForegroundWindow();
        if fg == 0 || fg == self.tip_hwnd || fg == self.msg_hwnd { self.hide_tip(); self.last_key = sentinel; self.force_until = 0.0; return; }
        if IsIconic(fg) != 0 { self.hide_tip(); self.last_key = sentinel; return; }
        if cfg.ignore_fullscreen && self.is_fullscreen(fg) { self.hide_tip(); self.last_key = sentinel; return; }
        let mut gti: GUITHREADINFO = unsafe { std::mem::zeroed() };
        gti.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        let tid = GetWindowThreadProcessId(fg, std::ptr::null_mut());
        let got = GetGUIThreadInfo(tid, &mut gti);
        let focus = if got != 0 && gti.hwndFocus != 0 { gti.hwndFocus } else { fg };
        let st = match detect_state(focus) {
            Some(s) => {
                self.last_ime_state = Some(s);
                s
            }
            None if in_force => {
                // 桌面/Chromium 等窗口可能不提供 IMM 状态接口。按键触发时仍显示上次状态；
                // 首次启动还没有缓存时，按当前键盘布局构造一个保守默认状态，不能静默不显示。
                let fallback = self.last_ime_state.unwrap_or_else(|| {
                    let lang = lang_id_of(focus);
                    (false, 1, lang, 0)
                });
                dbg(&format!("IME query unavailable; force hint with cached/default state {:?}", fallback));
                fallback
            }
            None => return,
        };
        let (is_cn, symbol, lang, _conv) = st;
        let caps = self.caps_state(); // 中文模式下大写也显示
        let mut caret = caret_screen(&gti);
        let mut mouse_live = false; // 位置是否来自真实鼠标指针(用于跟随续期)
        if caret.is_none() {
            // 无光标: 鼠标兑底 或 按键悬停窗口内 → 在鼠标位置显示
            if !cfg.follow_mouse && !in_force {
                // 光标暂不可用: 不立即隐藏, 给 400ms 宽限。
                // 部分程序(如 PowerPoint)的 hwndCaret 会间歇性为 0, 立即隐藏会造成闪烁。
                if self.no_caret_since == 0.0 { self.no_caret_since = t; dbg("caret gone"); }
                if t - self.no_caret_since > 0.4 {
                    // 同样保留 last_key, 避免光标可用性振荡造成闪烁
                    if self.visible { self.hide_tip(); }
                }
                return;
            }
            self.no_caret_since = 0.0;
            if in_force && !cfg.follow_mouse {
                // 按键触发: 固定在触发点(不是实时鼠标位置)
                let (px, py) = self.force_pos;
                caret = Some((px, py, py));
            } else {
                let mut pt = POINT { x: 0, y: 0 };
                if GetCursorPos(&mut pt) == 0 { return; }
                caret = Some((pt.x, pt.y, pt.y));
                mouse_live = true;
            }
        }
        self.no_caret_since = 0.0;
        let caret = caret.unwrap();
        // 状态键不含光标坐标: PPT 光标微动不能反复重置显示时间
        let state_key = (focus, is_cn, symbol, lang, caps);
        // 设置项外观有变化时重新渲染当前提示(无需等待下一次输入法切换)
        let cfg_changed = render_config_changed(self.last_render_cfg.as_ref(), &cfg);
        // 状态去抖: 同一新状态连续出现 2 次(约 100ms)才确认生效, 避免抖动导致反复重绘
        let mut state_changed = false;
        if state_key == self.last_key {
            self.pending_key = None;
            self.pending_count = 0;
        } else if self.pending_key == Some(state_key) {
            self.pending_count += 1;
            if self.pending_count >= 2 {
                state_changed = true;
                self.pending_key = None;
                self.pending_count = 0;
            }
        } else {
            self.pending_key = Some(state_key);
            self.pending_count = 1;
        }
        let caret_moved = (caret.0 - self.last_caret.0).abs()
            + (caret.1 - self.last_caret.1).abs() > 3; // >3px 才算移动, 滤除抖动
        let pending = t < self.pending_until;
        // 是否现在显示: 状态变化; 或按键触发后等待 IME 稳定(到期强制显示)
        let do_show = if pending { state_changed || cfg_changed || t >= self.pending_until } else { state_changed || cfg_changed };
        if cfg.show_on_change {
            if do_show {
                self.pending_until = 0.0;
                self.last_key = state_key;
                self.last_caret = (caret.0, caret.1);
                dbg(&format!("trigger key=({:#x},{},{},{},{}) caret=({},{}) pending={}",
                    focus as usize, is_cn, symbol, lang, caps, caret.0, caret.1, pending));
                self.show_tip(caret, build_glyphs(is_cn, symbol, lang, caps, &cfg), &cfg);
                self.show_until = t + cfg.timeout;
            } else if pending {
                // 正在等待 IME 状态稳定(保持当前显示不变)
            } else if self.visible {
                if t >= self.show_until {
                    // 超时隐藏: 保留 last_key(重置会导致下一轮立即重新触发, 造成周期性闪烁)
                    // 重新显示靠: 状态变化 / 光标移动(点击) / 按键触发
                    self.hide_tip();
                } else if caret_moved || mouse_live {
                    // 仅重新定位; 真实鼠标跟随才续期, 真实文本光标移动不续期
                    self.last_caret = (caret.0, caret.1);
                    if mouse_live { self.show_until = t + cfg.timeout; }
                    self.show_tip(caret, build_glyphs(is_cn, symbol, lang, caps, &cfg), &cfg);
                }
            } else if cfg_changed || caret_moved {
                // 当前提示已隐藏时, 外观变化或点击/光标移动才重新显示
                self.last_key = state_key;
                self.last_caret = (caret.0, caret.1);
                self.show_tip(caret, build_glyphs(is_cn, symbol, lang, caps, &cfg), &cfg);
                self.show_until = t + cfg.timeout;
            }
        } else {
            // 持续显示模式: 有光标就显示并跟随
            if do_show || cfg_changed || caret_moved || !self.visible {
                self.pending_until = 0.0;
                self.last_key = state_key;
                self.last_caret = (caret.0, caret.1);
                self.show_tip(caret, build_glyphs(is_cn, symbol, lang, caps, &cfg), &cfg);
            }
        }
    }
    unsafe fn make_tray_icon(&mut self) -> isize {
        const S: i32 = 32;
        let screen = GetDC(0);
        if screen == 0 { return 0; }
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = S;
        bmi.bmiHeader.biHeight = -S;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        let mut bits: *mut c_void = std::ptr::null_mut();
        let color = CreateDIBSection(screen, &bmi, 0, &mut bits, 0, 0);
        ReleaseDC(0, screen);
        if color == 0 || bits.is_null() { return 0; }

        // 透明底 + 对角渐变圆角方块(与 EXE 图标 tools/make_icon.py 的 teal 预设一致)
        let px = bits as *mut u8;
        for y in 0..S {
            for x in 0..S {
                let r = 7i32;
                let cx = if x < r { r } else if x >= S-r { S-r-1 } else { x };
                let cy = if y < r { r } else if y >= S-r { S-r-1 } else { y };
                let dx = x - cx; let dy = y - cy;
                let p = ((y * S + x) * 4) as isize;
                if dx*dx + dy*dy <= r*r {
                    // BGR 顺序写入(位图内存布局)
                    let t = (x + y) as f32 / ((S - 1) * 2) as f32;
                    let lerp = |a: u8, b: u8| -> u8 {
                        (a as f32 + (b as f32 - a as f32) * t).round() as u8
                    };
                    *px.offset(p) = lerp(ICON_C1.2, ICON_C2.2);     // B
                    *px.offset(p+1) = lerp(ICON_C1.1, ICON_C2.1);   // G
                    *px.offset(p+2) = lerp(ICON_C1.0, ICON_C2.0);   // R
                    *px.offset(p+3) = 0xFF;
                } else {
                    *px.offset(p+3) = 0;
                }
            }
        }

        let dc = CreateCompatibleDC(0);
        let old = SelectObject(dc, color as HGDIOBJ);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, 0x00FFFFFF);
        let name = wstr("Microsoft YaHei UI");
        let font = CreateFontW(-17, 0, 0, 0, FW_BOLD as i32, 0, 0, 0,
                               DEFAULT_CHARSET as u32, 0, 0, 0, 0, name.as_ptr());
        let oldf = SelectObject(dc, font as HGDIOBJ);
        let mut rc = RECT { left: 0, top: 1, right: S, bottom: S };
        let text = wstr("中");
        DrawTextW(dc, text.as_ptr(), -1, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        SelectObject(dc, oldf);
        SelectObject(dc, old);
        DeleteObject(font as HGDIOBJ);
        DeleteDC(dc);

        let mask = CreateBitmap(S, S, 1, 1, std::ptr::null());
        if std::env::var_os("IMTIP_DUMP").is_some() {
            // 诊断: 导出托盘图标(预乘 BGRA, 合成到白底), 便于核对配色
            dump_bmp_name(S, S, bits as *const u8, "imtip_tray.bmp");
        }
        let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info);
        DeleteObject(mask as HGDIOBJ);
        DeleteObject(color as HGDIOBJ);
        icon
    }

    unsafe fn tray_add(&mut self) {
        if self.tray_added { return; }
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.msg_hwnd; nid.uID = 1;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        if self.tray_icon == 0 { self.tray_icon = self.make_tray_icon(); }
        nid.hIcon = self.tray_icon;
        let tip = wstr("ImTipLite 输入法提示");
        let n = tip.len().min(128);
        nid.szTip[..n].copy_from_slice(&tip[..n]);
        if Shell_NotifyIconW(NIM_ADD, &nid) != 0 { self.tray_added = true; }
    }
    unsafe fn tray_del(&mut self) {
        if !self.tray_added { return; }
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.msg_hwnd; nid.uID = 1;
        Shell_NotifyIconW(NIM_DELETE, &nid);
        if self.tray_icon != 0 {
            DestroyIcon(self.tray_icon);
            self.tray_icon = 0;
        }
        self.tray_added = false;
    }
}

// ---- 开机自启(HKCU\...\Run) ----
const AUTOSTART_NAME: &str = "ImTipLite";
const AUTOSTART_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

unsafe fn exe_path() -> String {
    let mut buf = [0u16; 1024];
    let n = GetModuleFileNameW(0, buf.as_mut_ptr(), buf.len() as u32) as usize;
    String::from_utf16_lossy(&buf[..n.min(buf.len())])
}

unsafe fn open_run_key(access: u32) -> Option<HKEY> {
    let sub = wstr(AUTOSTART_KEY);
    let mut h: HKEY = 0;
    if RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, access, &mut h) == 0 && h != 0 {
        Some(h)
    } else {
        None
    }
}

/// 是否已设置开机自启
pub unsafe fn autostart_enabled() -> bool {
    let h = match open_run_key(KEY_QUERY_VALUE) { Some(h) => h, None => return false };
    let name = wstr(AUTOSTART_NAME);
    let mut ty: u32 = 0;
    let mut buf = [0u8; 1024];
    let mut cb: u32 = buf.len() as u32;
    let r = RegQueryValueExW(h, name.as_ptr(), std::ptr::null(), &mut ty,
                             buf.as_mut_ptr(), &mut cb);
    RegCloseKey(h);
    r == 0
}

/// 设置/取消开机自启(写入当前 exe 路径)
pub unsafe fn set_autostart(on: bool) -> bool {
    let h = match open_run_key(KEY_QUERY_VALUE | KEY_SET_VALUE) { Some(h) => h, None => return false };
    let name = wstr(AUTOSTART_NAME);
    let ok = if on {
        // 注册表命令: "C:\path\ImTipLite.exe"
        let cmd: Vec<u16> = format!("\"{}\"", exe_path())
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let bytes = std::slice::from_raw_parts(cmd.as_ptr() as *const u8, cmd.len() * 2);
        RegSetValueExW(h, name.as_ptr(), 0, REG_SZ, bytes.as_ptr(), bytes.len() as u32) == 0
    } else {
        // 2 = ERROR_FILE_NOT_FOUND, 视为已取消
        let r = RegDeleteValueW(h, name.as_ptr());
        r == 0 || r == 2
    };
    RegCloseKey(h);
    ok
}

// ---- 提示条光栅化(实际提示条与设置页预览共用, 保证外观完全一致) ----

/// 当前系统 DPI 缩放系数
pub unsafe fn dpi_scale_static() -> f64 {
    let hdc = GetDC(0);
    if hdc != 0 {
        let dpi = GetDeviceCaps(hdc, LOGPIXELSX as i32);
        ReleaseDC(0, hdc);
        if dpi > 0 { return dpi as f64 / 96.0; }
    }
    1.0
}

/// 按配置与单元格数计算提示条像素尺寸
pub fn tip_size(cfg: &Config, n_cells: i32) -> (i32, i32) {
    unsafe {
        let scale = dpi_scale_static();
        let pad = (4.0 * scale).round() as i32;
        let cell = (cfg.cell_w as f64 * scale).round() as i32;
        let h = (cfg.height as f64 * scale).round() as i32;
        (pad * 2 + cell * n_cells.max(1), h)
    }
}

/// 测量单格文字的真实墨迹外接矩形(ink box)。
///
/// 做法: 在与主画布同分辨率/同抗锯齿设置的临时透明画布上把文字画一次, 然后扫描
/// alpha 通道, 取所有非透明像素的外接矩形。GdipMeasureString 只能给出包含行距的
/// 排版框(汉字与半角点的排版框一样高, 但字形高度差别很大), 用它居中会让 "。" / "·"
/// 这类小字形明显偏下; 扫描得到的 ink box 才是真正的可视范围。
///
/// 返回 (left, top, right, bottom), 坐标相对临时画布左上角(含右/下边界)。
unsafe fn measure_text_ink(
    font: *mut GpFont,
    sf: *mut GpStringFormat,
    text: &[u16],
    cw: i32,
    ch: i32,
    argb: u32,
) -> Option<(i32, i32, i32, i32)> {
    if cw <= 0 || ch <= 0 || text.len() <= 1 { return None; }
    let mut bmi: BITMAPINFO = std::mem::zeroed();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = cw;
    bmi.bmiHeader.biHeight = -ch; // 自顶向下
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    let mut px: *mut c_void = std::ptr::null_mut();
    let screen = GetDC(0);
    let dib = CreateDIBSection(screen, &bmi, 0, &mut px, 0, 0);
    ReleaseDC(0, screen);
    if dib == 0 || px.is_null() { return None; }
    let mut bmp: *mut GpBitmap = std::ptr::null_mut();
    if GdipCreateBitmapFromScan0(cw, ch, cw * 4, PIXEL_FORMAT_32BPP_ARGB, px as *const u8, &mut bmp) != 0
        || bmp.is_null()
    {
        DeleteObject(dib as HGDIOBJ);
        return None;
    }
    let mut g: *mut GpGraphics = std::ptr::null_mut();
    if GdipGetImageGraphicsContext(bmp as *mut GpImage, &mut g) != 0 || g.is_null() {
        GdipDisposeImage(bmp as *mut GpImage);
        DeleteObject(dib as HGDIOBJ);
        return None;
    }
    GdipSetTextRenderingHint(g, 4); // AntiAlias, 与主画布一致
    GdipGraphicsClear(g, 0);
    let mut brush: *mut GpSolidFill = std::ptr::null_mut();
    let mut ink: Option<(i32, i32, i32, i32)> = None;
    if GdipCreateSolidFill(argb, &mut brush) == 0 {
        let rect = RectF { X: 0.0, Y: 0.0, Width: cw as f32, Height: ch as f32 };
        let st = GdipDrawString(g, text.as_ptr(), (text.len() as i32) - 1, font, &rect, sf, brush);
        GdipDeleteBrush(brush);
        if st == 0 {
            let p = px as *const u8;
            for y in 0..ch {
                for x in 0..cw {
                    let a = *p.offset(((y * cw + x) * 4 + 3) as isize);
                    if a >= INK_ALPHA_THRESHOLD {
                        ink = Some(match ink {
                            None => (x, y, x, y),
                            Some((l, t, r, b)) => (l.min(x), t.min(y), r.max(x), b.max(y)),
                        });
                    }
                }
            }
        }
    }
    GdipDeleteGraphics(g);
    GdipDisposeImage(bmp as *mut GpImage);
    DeleteObject(dib as HGDIOBJ);
    ink
}

/// 光栅化提示条: 4x 超采样绘制 -> 高质量缩小 -> 预乘 BGRA。
/// 返回 w*h*4 字节的预乘像素(可直接交给 UpdateLayeredWindow)。
pub unsafe fn rasterize_tip(w: i32, h: i32, cfg: &Config, cells: &[(String, (u8, u8, u8))]) -> Option<Vec<u8>> {
    if w <= 0 || h <= 0 { return None; }
    // 超采样倍率: 先在 4x 画布上渲染, 再高质量缩小 -> 圆角/文字边缘平滑无阶梯
    const SS: i32 = 4;
    let sw = w * SS;
    let sh = h * SS;

    let scale = dpi_scale_static();
    let alpha = cfg.alpha as u32;
    let r = (cfg.radius as f64 * scale).round() as i32;
    let bw = (cfg.border_width as f64 * scale).round() as i32;
    let bgc = parse_color(&cfg.color_bg, BG);
    let bg_argb = (alpha << 24) | (bgc.0 as u32) << 16 | (bgc.1 as u32) << 8 | bgc.2 as u32;

    // ---------- 1) 超采样画布 ----------
    let mut sbmi: BITMAPINFO = std::mem::zeroed();
    sbmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    sbmi.bmiHeader.biWidth = sw;
    sbmi.bmiHeader.biHeight = -sh; // 自顶向下
    sbmi.bmiHeader.biPlanes = 1;
    sbmi.bmiHeader.biBitCount = 32;
    let mut spx: *mut c_void = std::ptr::null_mut();
    let screen = GetDC(0);
    let sbm = CreateDIBSection(screen, &sbmi, 0, &mut spx, 0, 0);
    ReleaseDC(0, screen);
    if sbm == 0 || spx.is_null() { return None; }

    let mut sbmp: *mut GpBitmap = std::ptr::null_mut();
    if GdipCreateBitmapFromScan0(sw, sh, sw * 4, PIXEL_FORMAT_32BPP_ARGB, spx as *const u8, &mut sbmp) != 0 || sbmp.is_null() {
        DeleteObject(sbm as HGDIOBJ);
        return None;
    }
    let mut sg: *mut GpGraphics = std::ptr::null_mut();
    if GdipGetImageGraphicsContext(sbmp as *mut GpImage, &mut sg) != 0 || sg.is_null() {
        GdipDisposeImage(sbmp as *mut GpImage);
        DeleteObject(sbm as HGDIOBJ);
        return None;
    }
    GdipSetSmoothingMode(sg, 4);      // AntiAlias
    GdipSetTextRenderingHint(sg, 4);  // AntiAlias
    GdipGraphicsClear(sg, 0);         // 全透明

    // 圆角矩形多边形(超采样空间; GDI+ FillPath 在本环境恒返回 ObjectBusy, 故用多边形)
    // SS=4 超采样 + 16 段/角 → 等效 64 段弧, 边缘极平滑
    const CORNER_SAMPLES: i32 = 16;
    let pr = r * SS;
    let mut pts: Vec<Point> = Vec::with_capacity(4 + 4 * CORNER_SAMPLES as usize);
    if pr > 0 {
        let rf = pr as f64;
        let wf = sw as f64;
        let hf = sh as f64;
        pts.push(Point { X: pr, Y: 0 });
        pts.push(Point { X: (wf - rf).round() as i32, Y: 0 });
        let corners: [(f64, f64, f64); 4] = [
            (wf - rf, rf, 270.0),
            (wf - rf, hf - rf, 0.0),
            (rf, hf - rf, 90.0),
            (rf, rf, 180.0),
        ];
        for (cx, cy, a0) in corners {
            for k in 1..=CORNER_SAMPLES {
                let a = (a0 + 90.0 * k as f64 / CORNER_SAMPLES as f64).to_radians();
                pts.push(Point {
                    X: (cx + rf * a.cos()).round() as i32,
                    Y: (cy + rf * a.sin()).round() as i32,
                });
            }
        }
    } else {
        pts.push(Point { X: 0, Y: 0 });
        pts.push(Point { X: sw, Y: 0 });
        pts.push(Point { X: sw, Y: sh });
        pts.push(Point { X: 0, Y: sh });
    }
    // 背景 + 边框(透明背景模式下不绘制)
    if !cfg.transparent_bg {
        let mut brush: *mut GpSolidFill = std::ptr::null_mut();
        if GdipCreateSolidFill(bg_argb, &mut brush) == 0 {
            GdipFillPolygonI(sg, brush, pts.as_ptr(), pts.len() as i32, 1 /*Winding*/);
            GdipDeleteBrush(brush);
        }
        if bw > 0 {
            let bcol = parse_color(&cfg.color_border, (0xDB,0xA6,0x5A));
            let b_argb = (alpha << 24) | (bcol.0 as u32) << 16 | (bcol.1 as u32) << 8 | bcol.2 as u32;
            let mut pen: *mut GpPen = std::ptr::null_mut();
            if GdipCreatePen1(b_argb, (bw * SS) as f32, 2 /*UnitPixel*/, &mut pen) == 0 {
                GdipDrawPolygonI(sg, pen, pts.as_ptr(), pts.len() as i32);
                GdipDeletePen(pen);
            }
        }
    }

    // 文本(字体与单元格同样按 SS 放大)
    let font_px = (cfg.font_size as f64 * scale).round() as i32;
    let fname = wstr(&cfg.font_name);
    let sfont = CreateFontW(-(font_px * SS), 0, 0, 0, FW_BOLD as i32, 0, 0, 0,
                            DEFAULT_CHARSET as u32, 0, 0, 0, 0, fname.as_ptr());
    let sdc = CreateCompatibleDC(0);
    let soldf = SelectObject(sdc, sfont as HGDIOBJ);
    let mut sf: *mut GpStringFormat = std::ptr::null_mut();
    GdipStringFormatGetGenericDefault(&mut sf);
    GdipSetStringFormatAlign(sf, 1 /*Center*/);
    GdipSetStringFormatLineAlign(sf, 0 /*Near*/);
    // 单格文字禁止折行/裁剪, 保证任何字符(包括多字符的 char_other)都完整绘制
    GdipSetStringFormatFlags(sf, STRING_FORMAT_NOWRAP_NOCLIP);
    let mut gfont: *mut GpFont = std::ptr::null_mut();
    let font_status = GdipCreateFontFromDC(sdc, &mut gfont);
    dbg(&format!("font create status={} null={} cells={} font_px={}", font_status, gfont.is_null(), cells.len(), font_px));
    if font_status == 0 && !gfont.is_null() {
        let pad = (4.0 * scale).round() as i32 * SS;
        let cellw = (cfg.cell_w as f64 * scale).round() as i32 * SS;
        // 测量画布比单元格宽: GDI+ 即使带 NoClip 也会截断超宽字符串,
        // 必须先把文字完整画在一个足够宽的画布上才能拿到真实的字形外接框。
        let measure_w = (cellw * 4).max(sh);
        for (i, (text, color)) in cells.iter().enumerate() {
            let tw = wstr(text);
            let argb = 0xFF000000u32 | (color.0 as u32) << 16 | (color.1 as u32) << 8 | color.2 as u32;
            let cell_x = pad + (i as i32) * cellw;
            // 可用区域(留 1 个超采样像素边距, 避免抗锯齿贴边)
            let avail_w = (cellw - 2 * SS).max(4) as f32;
            let avail_h = (sh - 2 * SS).max(4) as f32;
            // 1) GdipMeasureString: 排版尺寸(含行距) —— 用于空字符判定与字体缩放决策
            let mut layout: RectF = std::mem::zeroed();
            let mut fitted: i32 = 0;
            let mut nlines: i32 = 0;
            let probe = RectF { X: 0.0, Y: 0.0, Width: measure_w as f32, Height: sh as f32 };
            let mst = GdipMeasureString(sg, tw.as_ptr(), (tw.len() as i32) - 1,
                gfont, &probe, sf, &mut layout, &mut fitted, &mut nlines);
            if mst != 0 || layout.Width <= 0.0 {
                dbg(&format!("measure empty text={:?} mst={} w={:.1}", text, mst, layout.Width));
                continue; // 空白字符: 不画, 也不占用布局
            }
            // 2) 量出真实字形包围盒; 超出单元格时等比缩小字体后重量(最多 2 轮)
            let mut cell_font = gfont;
            let mut extra: Option<(*mut GpFont, isize, isize)> = None; // (font, dc, gdifont)
            let mut ink = measure_text_ink(cell_font, sf, &tw, measure_w, sh, argb);
            for _ in 0..2 {
                let Some((l, t, r, b)) = ink else { break };
                let ink_w = (r - l + 1) as f32;
                let ink_h = (b - t + 1) as f32;
                let k = (avail_w / ink_w).min(avail_h / ink_h).min(1.0);
                if k >= 0.99 { break; }
                // 释放上一轮缩小的字体
                if let Some((f, dc, gd)) = extra.take() {
                    GdipDeleteFont(f);
                    DeleteDC(dc);
                    DeleteObject(gd as HGDIOBJ);
                }
                let fit_px = (((font_px * SS) as f32) * k).floor() as i32;
                let fit_px = fit_px.max(8);
                let gd = CreateFontW(-fit_px, 0, 0, 0, FW_BOLD as i32, 0, 0, 0,
                    DEFAULT_CHARSET as u32, 0, 0, 0, 0, fname.as_ptr());
                let dc = CreateCompatibleDC(0);
                let mut nf: *mut GpFont = std::ptr::null_mut();
                if gd == 0 || dc == 0 {
                    if dc != 0 { DeleteDC(dc); }
                    if gd != 0 { DeleteObject(gd as HGDIOBJ); }
                    break;
                }
                SelectObject(dc, gd as HGDIOBJ);
                if GdipCreateFontFromDC(dc, &mut nf) != 0 || nf.is_null() {
                    DeleteDC(dc);
                    DeleteObject(gd as HGDIOBJ);
                    break;
                }
                cell_font = nf;
                extra = Some((nf, dc, gd));
                ink = measure_text_ink(cell_font, sf, &tw, measure_w, sh, argb);
            }
            // 3) 把字形像素范围的中心对齐到单元格中心(中/英/全角/半角/A 同一算法)
            let draw_rect = match ink {
                Some((l, t, r, b)) => {
                    // 水平与垂直都用同一个墨迹包围盒中心对齐到单元格中心。
                    // 不能依赖 GDI+ 的 Center 对齐: 它按字宽度(advance)居中,
                    // 句号这类字形在 em 框内偏侧, 用 advance 居中会明显偏。
                    let ink_cx = (l + r + 1) as f32 * 0.5;
                    let ink_cy = (t + b + 1) as f32 * 0.5;
                    RectF {
                        X: (cell_x as f32 + cellw as f32 * 0.5 - ink_cx).round(),
                        Y: (sh as f32 * 0.5 - ink_cy).round(),
                        Width: measure_w as f32,
                        Height: sh as f32,
                    }
                }
                None => {
                    // 墨迹扫描失败: 退回 GdipMeasureString 的排版框居中
                    RectF {
                        X: (cell_x as f32 + cellw as f32 * 0.5 - layout.Width * 0.5).round(),
                        Y: (sh as f32 * 0.5 - layout.Height * 0.5).round(),
                        Width: measure_w as f32,
                        Height: sh as f32,
                    }
                }
            };
            let mut tb: *mut GpSolidFill = std::ptr::null_mut();
            if GdipCreateSolidFill(argb, &mut tb) == 0 {
                let draw_status = GdipDrawString(sg, tw.as_ptr(), (tw.len() as i32) - 1, cell_font, &draw_rect, sf, tb);
                GdipDeleteBrush(tb);
                dbg(&format!("draw text={:?} status={} ink={:?} rect=({},{} {}x{}) shrunk={}",
                    text, draw_status, ink, draw_rect.X, draw_rect.Y, draw_rect.Width, draw_rect.Height,
                    extra.is_some()));
            } else {
                dbg("GdipCreateSolidFill(text) failed");
            }
            if let Some((f, dc, gd)) = extra.take() {
                GdipDeleteFont(f);
                DeleteDC(dc);
                DeleteObject(gd as HGDIOBJ);
            }
        }
        GdipDeleteFont(gfont);
    } else {
        dbg(&format!("font create failed status={} null={}", font_status, gfont.is_null()));
    }
    GdipDeleteStringFormat(sf);
    SelectObject(sdc, soldf);
    DeleteObject(sfont as HGDIOBJ);
    DeleteDC(sdc);
    GdipDeleteGraphics(sg);

    // ---------- 2) 目标画布: 高质量缩小 ----------
    let mut tbmi: BITMAPINFO = std::mem::zeroed();
    tbmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    tbmi.bmiHeader.biWidth = w;
    tbmi.bmiHeader.biHeight = -h;
    tbmi.bmiHeader.biPlanes = 1;
    tbmi.bmiHeader.biBitCount = 32;
    let mut tpx: *mut c_void = std::ptr::null_mut();
    let screen2 = GetDC(0);
    let tbm = CreateDIBSection(screen2, &tbmi, 0, &mut tpx, 0, 0);
    ReleaseDC(0, screen2);
    if tbm == 0 || tpx.is_null() {
        GdipDisposeImage(sbmp as *mut GpImage);
        DeleteObject(sbm as HGDIOBJ);
        return None;
    }
    let mut dbmp: *mut GpBitmap = std::ptr::null_mut();
    if GdipCreateBitmapFromScan0(w, h, w * 4, PIXEL_FORMAT_32BPP_ARGB, tpx as *const u8, &mut dbmp) == 0 && !dbmp.is_null() {
        let mut dg: *mut GpGraphics = std::ptr::null_mut();
        if GdipGetImageGraphicsContext(dbmp as *mut GpImage, &mut dg) == 0 && !dg.is_null() {
            GdipSetInterpolationMode(dg, 7); // HighQualityBicubic
            GdipSetPixelOffsetMode(dg, 2);   // HighQuality
            GdipSetCompositingMode(dg, 0);   // SourceOver
            GdipGraphicsClear(dg, 0);
            GdipDrawImageRectI(dg, sbmp as *mut GpImage, 0, 0, w, h);
            GdipDeleteGraphics(dg);
        }
        GdipDisposeImage(dbmp as *mut GpImage);
    }
    GdipDisposeImage(sbmp as *mut GpImage);
    DeleteObject(sbm as HGDIOBJ);

    // 拷贝出来并预乘(ULW 需要)
    let n = (w * h * 4) as usize;
    let mut out = vec![0u8; n];
    std::ptr::copy_nonoverlapping(tpx as *const u8, out.as_mut_ptr(), n);
    DeleteObject(tbm as HGDIOBJ);
    let mut i: usize = 0;
    while i < n {
        let a = out[i + 3] as u32;
        if a < 255 {
            out[i] = (out[i] as u32 * a / 255) as u8;
            out[i + 1] = (out[i + 1] as u32 * a / 255) as u8;
            out[i + 2] = (out[i + 2] as u32 * a / 255) as u8;
        }
        i += 4;
    }
    Some(out)
}

/// 预乘 BGRA -> 24bpp BMP, 合成到指定背景色(用于设置页预览)
pub fn bmp_from_premul(w: i32, h: i32, px: &[u8], br: u8, bgc: u8, bb: u8) -> Vec<u8> {
    let row = ((w * 3 + 3) / 4) * 4;
    let size = 54 + row * h;
    let mut out = vec![0u8; size as usize];
    out[0] = b'B'; out[1] = b'M';
    out[2..6].copy_from_slice(&(size as u32).to_le_bytes());
    out[10..14].copy_from_slice(&54u32.to_le_bytes());
    out[14..18].copy_from_slice(&40u32.to_le_bytes());
    out[18..22].copy_from_slice(&(w as u32).to_le_bytes());
    out[22..26].copy_from_slice(&(-h).to_le_bytes());
    out[26..28].copy_from_slice(&1u16.to_le_bytes());
    out[28..30].copy_from_slice(&24u16.to_le_bytes());
    for y in 0..h {
        for x in 0..w {
            let o = ((y * w + x) * 4) as usize;
            let a = px[o + 3] as u32;
            let inv = 255 - a;
            let b = px[o] as u32;
            let g = px[o + 1] as u32;
            let r = px[o + 2] as u32;
            let off = (54 + y * row + x * 3) as usize;
            out[off] = (b + bb as u32 * inv / 255).min(255) as u8;
            out[off + 1] = (g + bgc as u32 * inv / 255).min(255) as u8;
            out[off + 2] = (r + br as u32 * inv / 255).min(255) as u8;
        }
    }
    out
}

/// 极简 base64 编码(避免额外依赖)
fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity((data.len() + 2) / 3 * 4);
    for c in data.chunks(3) {
        let b0 = c[0] as u32;
        let b1 = *c.get(1).unwrap_or(&0) as u32;
        let b2 = *c.get(2).unwrap_or(&0) as u32;
        let v = (b0 << 16) | (b1 << 8) | b2;
        s.push(T[((v >> 18) & 63) as usize] as char);
        s.push(T[((v >> 12) & 63) as usize] as char);
        s.push(if c.len() > 1 { T[((v >> 6) & 63) as usize] as char } else { '=' });
        s.push(if c.len() > 2 { T[(v & 63) as usize] as char } else { '=' });
    }
    s
}

/// 用给定配置离屏渲染三张示例提示条(中文全角 / 中文大写 / 英文半角)并编码为 data URL。
/// 与真实提示条走同一 rasterize_tip, 因此外观完全一致。
pub fn preview_data_urls(cfg: &Config) -> Vec<String> {
    unsafe {
        let make = |cells: &[(String, (u8, u8, u8))]| -> String {
            let (w, h) = tip_size(cfg, cells.len() as i32);
            match rasterize_tip(w, h, cfg, cells) {
                Some(buf) => {
                    // 合成到浅灰底, 便于辨别浅色/透明背景
                    let bmp = bmp_from_premul(w, h, &buf, 0xEC, 0xEF, 0xF4);
                    format!("data:image/bmp;base64,{}", b64(&bmp))
                }
                None => String::new(),
            }
        };
        let cell_cn = || (if cfg.char_cn.is_empty() { "中".to_string() } else { cfg.char_cn.clone() },
                          parse_color(&cfg.color_cn, COLOR_CN));
        let cell_en = || (if cfg.char_en.is_empty() { "英".to_string() } else { cfg.char_en.clone() },
                          parse_color(&cfg.color_en, COLOR_EN));
        let cell_full = || (if cfg.char_full.is_empty() { "。".to_string() } else { cfg.char_full.clone() },
                            parse_color(&cfg.color_full, COLOR_SYMBOL));
        let cell_half = || (if cfg.char_half.is_empty() { "·".to_string() } else { cfg.char_half.clone() },
                            parse_color(&cfg.color_half, COLOR_HALF));
        let cell_caps = || (if cfg.char_caps.is_empty() { "A".to_string() } else { cfg.char_caps.clone() },
                            parse_color(&cfg.color_caps, COLOR_CAPS));
        let cn_full = vec![cell_cn(), cell_full()];
        let cn_caps = vec![cell_cn(), cell_caps()];
        let en_half = vec![cell_en(), cell_half()];
        vec![make(&cn_full), make(&cn_caps), make(&en_half)]
    }
}

static mut APP: Option<App> = None;
thread_local! { static IN_CALL: Cell<bool> = const { Cell::new(false) }; }

/// CapsLock 开关状态(由低级键盘钩子跟踪, 跨进程可靠)。
static CAPS_ON: AtomicBool = AtomicBool::new(false);
/// 去重键盘自动重复: 仅首次 keydown 切换 Caps 状态, keyup 解锁下一次切换
static CAPS_HELD: AtomicBool = AtomicBool::new(false);
/// 按键触发的立即显示请求(Shift/CapsLock 按下时置位, 下一次轮询立刻弹窗)。
static SHOW_HINT: AtomicBool = AtomicBool::new(false);
const VK_LSHIFT: u32 = 0xA0;
const VK_RSHIFT: u32 = 0xA1;

unsafe extern "system" fn ll_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == 0 /*HC_ACTION*/ {
        let msg = wp as u32;
        let kb = &*(lp as *const KBDLLHOOKSTRUCT);
        let is_keydown = msg == 0x100 /*WM_KEYDOWN*/ || msg == 0x104 /*WM_SYSKEYDOWN*/;
        let is_keyup = msg == 0x101 /*WM_KEYUP*/ || msg == 0x105 /*WM_SYSKEYUP*/;
        if is_keydown && kb.vkCode == VK_CAPITAL as u32 {
            if !CAPS_HELD.swap(true, Ordering::Relaxed) {
                let on = CAPS_ON.fetch_xor(true, Ordering::Relaxed);
                SHOW_HINT.store(true, Ordering::Relaxed);
                dbg(&format!("caps down -> {}", !on));
            }
        } else if is_keyup && kb.vkCode == VK_CAPITAL as u32 {
            CAPS_HELD.store(false, Ordering::Relaxed);
        } else if is_keyup && (kb.vkCode == VK_LSHIFT || kb.vkCode == VK_RSHIFT) {
            // 中英切换在 Shift 抬起时才完成, 故在抬起时触发(避免显示旧状态)
            SHOW_HINT.store(true, Ordering::Relaxed);
            dbg("shift up");
        }
    }
    CallNextHookEx(0, code, wp, lp)
}

unsafe fn init_caps() {
    // 启动时用一次 AttachThreadInput+GetKeyState 取初始状态(随后由钩子维护)
    let fg = GetForegroundWindow();
    if fg != 0 {
        let tid = GetWindowThreadProcessId(fg, std::ptr::null_mut());
        let me = GetCurrentThreadId();
        if tid != 0 && tid != me && AttachThreadInput(me, tid, 1) != 0 {
            let r = GetKeyState(VK_CAPITAL as i32) & 1;
            AttachThreadInput(me, tid, 0);
            CAPS_ON.store(r != 0, Ordering::Relaxed);
        }
    }
}

unsafe extern "system" fn msg_wndproc(hwnd: isize, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wp == TIMER_POLL => {
            let reenter = IN_CALL.with(|f| f.replace(true));
            if !reenter {
                if let Some(a) = APP.as_mut() {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { a.poll(); }));
                }
            }
            IN_CALL.with(|f| f.set(false));
            0
        }
        WM_TRAY => {
            let m = lp as u32 & 0xFFFF;
            if m == WM_LBUTTONUP || m == WM_LBUTTONDBLCLK {
                // 左键单击 / 双击托盘图标 → 打开设置窗口(双击也可从桌面双击 exe 触发)
                if let Some(f) = SHOW_SETTINGS.get() { f(); }
                return 0;
            }
            if m == WM_RBUTTONUP {
                let snap = if let Some(a) = APP.as_ref() { a.snap() } else { return 0; };
                let msg_hwnd = if let Some(a) = APP.as_ref() { a.msg_hwnd } else { return 0; };
                // Windows 托盘菜单：先置前，确保右键菜单能正常收起
                SetForegroundWindow(msg_hwnd);
                let menu = CreatePopupMenu();
                let flag = |b: bool| if b { MF_CHECKED } else { 0 };
                let s1 = wstr(if snap.paused { "恢复输入提示(&P)" } else { "暂停输入提示(&P)" });
                AppendMenuW(menu, MF_STRING | flag(snap.paused), IDM_PAUSE, s1.as_ptr());
                let s2 = wstr("鼠标位置兜底(兼容, &M)");
                AppendMenuW(menu, MF_STRING | flag(snap.follow_mouse), IDM_MOUSE, s2.as_ptr());
                let s3 = wstr("忽略全屏窗口(&F)");
                AppendMenuW(menu, MF_STRING | flag(snap.ignore_fullscreen), IDM_FULLSCREEN, s3.as_ptr());
                AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                let s4 = wstr("打开设置(&S)");
                AppendMenuW(menu, MF_STRING, IDM_SHOW, s4.as_ptr());
                let s5 = wstr("退出 ImTipLite(&X)");
                AppendMenuW(menu, MF_STRING, IDM_EXIT, s5.as_ptr());
                let mut pt = POINT { x:0, y:0 }; GetCursorPos(&mut pt);
                let cmd = TrackPopupMenu(menu, TPM_RIGHTBUTTON|TPM_RETURNCMD, pt.x, pt.y, 0, msg_hwnd, std::ptr::null());
                PostMessageW(msg_hwnd, WM_NULL, 0, 0);
                DestroyMenu(menu);
                match cmd as usize {
                    IDM_PAUSE => { if let Some(a) = APP.as_mut() { let mut c=a.cfg.lock().unwrap(); c.paused=!c.paused; } if let Some(a)=APP.as_mut(){a.hide_tip();} }
                    IDM_MOUSE => { if let Some(a) = APP.as_mut() { let mut c=a.cfg.lock().unwrap(); c.follow_mouse=!c.follow_mouse; } }
                    IDM_FULLSCREEN => { if let Some(a) = APP.as_mut() { let mut c=a.cfg.lock().unwrap(); c.ignore_fullscreen=!c.ignore_fullscreen; } }
                    IDM_SHOW => { if let Some(f) = SHOW_SETTINGS.get() { f(); } }
                    IDM_EXIT => { if let Some(f) = EXIT_APP.get() { f(); } }
                    _ => {}
                }
            }
            0
        }
        WM_SHOW_SETTINGS => {
            // 第二个实例请求把设置窗口调到前面
            if let Some(f) = SHOW_SETTINGS.get() { f(); }
            0
        }
        WM_DESTROY => {
            if let Some(a) = APP.as_mut() {
                a.tray_del(); KillTimer(a.msg_hwnd, TIMER_POLL);
                if a.tip_hwnd != 0 { DestroyWindow(a.tip_hwnd); a.tip_hwnd = 0; }
            }
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
unsafe extern "system" fn tip_wndproc(hwnd: isize, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            // 内容由 render() + UpdateLayeredWindow 驱动, 这里只需验证空白区避免无休止重绘
            ValidateRect(hwnd, std::ptr::null());
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// 在新线程里运行原生提示核心。config 为共享配置。
pub fn run_tip_thread(cfg: Arc<Mutex<Config>>) {
    std::thread::Builder::new()
        .name("imtip-tip".into())
        .spawn(move || unsafe { run(cfg) })
        .expect("spawn tip thread");
}

unsafe fn run(cfg: Arc<Mutex<Config>>) {
    let hinst = GetModuleHandleW(std::ptr::null()) as HMODULE as isize;

    // GDI+ 初始化(逐像素 alpha 渲染用)
    let mut token: usize = 0;
    let mut gp_input: GdiplusStartupInput = std::mem::zeroed();
    gp_input.GdiplusVersion = 1;
    let rc = GdiplusStartup(&mut token, &gp_input, std::ptr::null_mut());
    dbg(&format!("GdiplusStartup rc={}", rc));

    APP = Some(App::new(hinst, cfg));
    let cls = wstr("ImTipLiteMsg");
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, style: CS_HREDRAW_VREDRAW,
        lpfnWndProc: Some(msg_wndproc), cbClsExtra:0, cbWndExtra:0,
        hInstance: hinst as HINSTANCE, hIcon:0, hCursor: LoadCursorW(0, IDC_ARROW),
        hbrBackground:0, lpszMenuName: std::ptr::null(), lpszClassName: cls.as_ptr(), hIconSm:0,
    };
    RegisterClassExW(&wc);
    let hwnd = CreateWindowExW(0, cls.as_ptr(), wstr("ImTipLite").as_ptr(), WS_POPUP, 0,0,0,0, 0,0, hinst as HINSTANCE, std::ptr::null());
    if hwnd == 0 { return; }
    let interval = if let Some(a) = APP.as_ref() { a.snap().poll_interval } else { 50 };
    if let Some(a) = APP.as_mut() { a.msg_hwnd = hwnd; a.timer_interval = interval; a.tray_add(); }
    SetTimer(hwnd, TIMER_POLL, interval.max(10), None);

    // CapsLock: 安装低级键盘钩子跟踪(需消息循环, 已在本线程)
    init_caps();
    let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_hook), hinst as HINSTANCE, 0);
    let _ = hook; // 钩子随线程/进程退出自动释放

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        let r = GetMessageW(&mut msg, 0, 0, 0);
        if r <= 0 { break; }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    // 进程退出,APP 自然回收(GDI 由 OS 回收)
    let _ = GetLastError();
}
