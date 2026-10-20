// ImTipLite (Rust 原生版) —— 方案 A 的原生核心
// 检测原理与 aardio key.ime.state 一致:
//   WM_IME_CONTROL(0x0283) -> IMC_GETOPENSTATUS(5) / IMC_GETCONVERSIONMODE(1)
//   经默认 IME 窗口跨进程获取输入法状态。
// 提示条为原生 Win32 layered 窗口 + GDI 绘制(不走 WebView)。

#![allow(non_snake_case, non_upper_case_globals)]

use std::cell::Cell;
use std::ffi::c_void;

use windows_sys::Win32::Foundation::{
    GetLastError, BOOL, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NOTIFYICONDATAW, NIM_ADD, NIM_DELETE, NIF_ICON, NIF_MESSAGE, NIF_TIP,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyboardLayout, VK_CAPITAL,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

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
const TIMER_POLL: usize = 1;
const WM_IME_CONTROL: u32 = 0x0283;
const IMC_GETOPENSTATUS: WPARAM = 0x0005;
const IMC_GETCONVERSIONMODE: WPARAM = 0x0001;
const SMTO_ABORTIFHUNG: u32 = 0x0002;
const ERROR_ALREADY_EXISTS: u32 = 183;
const CS_HREDRAW_VREDRAW: u32 = 0x0003;

const IDM_PAUSE: usize = 1001;
const IDM_MOUSE: usize = 1002;
const IDM_EXIT: usize = 1003;

const POLL_MS: u32 = 50;
const TIMEOUT: f64 = 2.0;
const FOLLOW_MOUSE_DEFAULT: bool = false;
const IGNORE_FULLSCREEN: bool = true;

const BG: (u8, u8, u8) = (0x26, 0x2B, 0x33);
const COLOR_CN: (u8, u8, u8) = (0x6B, 0xE5, 0x85);
const COLOR_EN: (u8, u8, u8) = (0x6B, 0xB6, 0xFF);
const COLOR_SYMBOL: (u8, u8, u8) = (0xFF, 0xB4, 0x54);
const COLOR_CAPS: (u8, u8, u8) = (0xFF, 0xD6, 0x6B);
const COLOR_HALF: (u8, u8, u8) = (0x7A, 0x82, 0x91);

const CJK_LANG_IDS: &[u32] = &[0x0804, 0x0404, 0x0C04, 0x1404, 0x0412, 0x0411];

fn rgb(c: (u8, u8, u8)) -> u32 {
    (c.0 as u32) | ((c.1 as u32) << 8) | ((c.2 as u32) << 16)
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

// ---- 检测 ----
unsafe fn ime_control(hwnd: isize, cmd: WPARAM) -> Option<usize> {
    let ime = ImmGetDefaultIMEWnd(hwnd);
    if ime == 0 {
        return None;
    }
    let mut res: usize = 0;
    let ok = SendMessageTimeoutW(
        ime,
        WM_IME_CONTROL,
        cmd,
        0,
        SMTO_ABORTIFHUNG,
        60,
        &mut res as *mut usize,
    );
    if ok == 0 {
        None
    } else {
        Some(res)
    }
}

unsafe fn lang_id_of(hwnd: isize) -> u32 {
    let tid = GetWindowThreadProcessId(hwnd, std::ptr::null_mut());
    let hkl = GetKeyboardLayout(tid);
    (hkl as u32) & 0xFFFF
}

unsafe fn caps_lock() -> i32 {
    // 不用 AttachThreadInput(会把前台输入队列挂上来,导致前台应用卡顿)。
    // GetAsyncKeyState 的低位对 CapsLock 切换键表示当前开关状态。
    (GetAsyncKeyState(VK_CAPITAL as i32) & 1) as i32
}

/// (is_cn, symbol_mode, lang_id, conv)。None 表示状态未知。
unsafe fn detect_state(focus: isize) -> Option<(bool, i32, u32, u32)> {
    let opened = ime_control(focus, IMC_GETOPENSTATUS);
    let conv = ime_control(focus, IMC_GETCONVERSIONMODE);
    if opened.is_none() && conv.is_none() {
        return None;
    }
    let lang = lang_id_of(focus);
    let mut opened = opened.unwrap_or(0) != 0;
    if opened && lang == 0x0409 {
        opened = false;
    }
    let conv = match conv {
        Some(c) => c as u32,
        None => return Some((false, 0, lang, 0)),
    };
    let mut symbol: i32 = 1;
    if !opened && (conv == 0 || conv == 1) && !CJK_LANG_IDS.contains(&lang) {
        symbol = 0;
    } else if (conv & 0x400) != 0 && opened {
        symbol = 3;
    } else if (conv & 0x0008) != 0 {
        symbol = 2;
    } else if (conv & 0x0100) != 0 {
        opened = false;
    }
    let is_cn = opened && (conv & 0x0003) != 0;
    Some((is_cn, symbol, lang, conv))
}

unsafe fn caret_screen(gti: &GUITHREADINFO) -> Option<(i32, i32, i32)> {
    if gti.hwndCaret == 0 || (gti.flags & 0x0001) == 0 {
        return None;
    }
    let mut bottom = POINT {
        x: gti.rcCaret.left,
        y: gti.rcCaret.bottom,
    };
    if ClientToScreen(gti.hwndCaret, &mut bottom) == 0 {
        return None;
    }
    let mut top = POINT { x: 0, y: gti.rcCaret.top };
    ClientToScreen(gti.hwndCaret, &mut top);
    Some((bottom.x, bottom.y, top.y))
}

fn build_glyphs(is_cn: bool, symbol: i32, lang: u32, caps: i32) -> Vec<(&'static str, (u8, u8, u8))> {
    let mut cells = Vec::new();
    let t1 = if CJK_LANG_IDS.contains(&lang) {
        if is_cn { "中" } else { "英" }
    } else if lang == 0x0411 {
        "あ"
    } else if lang == 0x0412 {
        "가"
    } else {
        "En"
    };
    let c1 = if is_cn { COLOR_CN } else { COLOR_EN };
    cells.push((t1, c1));
    let t2;
    let mut c2 = COLOR_HALF;
    if caps != 0 && !is_cn {
        t2 = "A";
        c2 = COLOR_CAPS;
    } else if symbol == 3 {
        t2 = "。";
        c2 = COLOR_SYMBOL;
    } else if symbol == 2 {
        t2 = "●";
        c2 = COLOR_SYMBOL;
    } else if symbol == 1 {
        t2 = "·";
    } else {
        t2 = "";
    }
    if !t2.is_empty() {
        cells.push((t2, c2));
    }
    cells
}

// ---- 应用状态 ----
struct App {
    msg_hwnd: isize,
    tip_hwnd: isize,
    visible: bool,
    show_until: f64,
    last_key: (isize, bool, i32, u32, i32),
    paused: bool,
    follow_mouse: bool,
    font: isize,
    font_px: i32,
    draw_cells: Vec<(&'static str, (u8, u8, u8))>,
    draw_w: i32,
    draw_h: i32,
    // 绘图缓存(避免每帧重建 DC/位图)
    mem_dc: isize,
    mem_bmp: isize,
    mem_w: i32,
    mem_h: i32,
    // 上次显示的签名(用于只在变化时重绘/重设区域)
    last_sig: (i32, i32, i32, u32, usize),
    hinst: isize,
    tray_added: bool,
}

impl App {
    fn new(hinst: isize) -> Self {
        App {
            msg_hwnd: 0,
            tip_hwnd: 0,
            visible: false,
            show_until: 0.0,
            last_key: (0, false, 0, 0, 0),
            paused: false,
            follow_mouse: FOLLOW_MOUSE_DEFAULT,
            font: 0,
            font_px: 0,
            draw_cells: Vec::new(),
            draw_w: 0,
            draw_h: 0,
            mem_dc: 0,
            mem_bmp: 0,
            mem_w: 0,
            mem_h: 0,
            last_sig: (-1, 0, 0, 0, 0),
            hinst,
            tray_added: false,
        }
    }

    unsafe fn hide_tip(&mut self) {
        if self.visible && self.tip_hwnd != 0 {
            ShowWindow(self.tip_hwnd, SW_HIDE);
        }
        self.visible = false;
    }

    unsafe fn ensure_tip(&mut self) {
        if self.tip_hwnd != 0 {
            return;
        }
        let cls = wstr("ImTipLiteTip");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW_VREDRAW,
            lpfnWndProc: Some(tip_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: self.hinst as HINSTANCE,
            hIcon: 0,
            hCursor: LoadCursorW(0, IDC_ARROW),
            hbrBackground: 0,
            lpszMenuName: std::ptr::null(),
            lpszClassName: cls.as_ptr(),
            hIconSm: 0,
        };
        RegisterClassExW(&wc);
        let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW
            | WS_EX_NOACTIVATE;
        self.tip_hwnd = CreateWindowExW(
            ex,
            cls.as_ptr(),
            std::ptr::null(),
            WS_POPUP,
            0, 0, 10, 10,
            0, 0, self.hinst as HINSTANCE, std::ptr::null(),
        );
        if self.tip_hwnd != 0 {
            SetLayeredWindowAttributes(self.tip_hwnd, 0, 235, LWA_ALPHA);
        }
    }

    unsafe fn dpi_scale(&self) -> f64 {
        let hdc = GetDC(0);
        if hdc != 0 {
            let dpi = GetDeviceCaps(hdc, LOGPIXELSX as i32);
            ReleaseDC(0, hdc);
            if dpi > 0 {
                return dpi as f64 / 96.0;
            }
        }
        1.0
    }

    unsafe fn show_tip(
        &mut self,
        fg: isize,
        caret: (i32, i32, i32),
        cells: Vec<(&'static str, (u8, u8, u8))>,
    ) {
        self.ensure_tip();
        if self.tip_hwnd == 0 {
            return;
        }
        let scale = self.dpi_scale();
        let pad = (4.0 * scale).round() as i32;
        let cell = (26.0 * scale).round() as i32;
        let h = (30.0 * scale).round() as i32;
        let n = cells.len().max(1) as i32;
        let w = pad * 2 + cell * n;
        let font_px = (17.0 * scale).round() as i32;
        if font_px != self.font_px || self.font == 0 {
            if self.font != 0 {
                DeleteObject(self.font);
            }
            let fname = wstr("Microsoft YaHei UI");
            self.font = CreateFontW(
                -font_px,
                0,
                0,
                0,
                FW_BOLD as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                0,
                0,
                fname.as_ptr(),
            );
            self.font_px = font_px;
        }

        let (cx, cy_bottom, cy_top) = caret;
        let mut x = cx + (14.0 * scale).round() as i32;
        let mut y = cy_bottom + (2.0 * scale).round() as i32;

        let mut mi: MONITORINFO = unsafe { std::mem::zeroed() };
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let mon = MonitorFromPoint(POINT { x: cx, y: cy_bottom }, MONITOR_DEFAULTTONEAREST as u32);
        if mon != 0 && GetMonitorInfoW(mon, &mut mi) != 0 {
            let work = mi.rcWork;
            if y + h > work.bottom {
                y = cy_top - h - (2.0 * scale).round() as i32;
            }
            if x + w > work.right {
                x = work.right - w - 4;
            }
            if x < work.left {
                x = work.left;
            }
            if y < work.top {
                y = work.top;
            }
        }

        let size_changed = w != self.draw_w || h != self.draw_h;
        let content_changed = cells != self.draw_cells;
        self.draw_cells = cells;
        self.draw_w = w;
        self.draw_h = h;
        if size_changed {
            let r = (9.0 * scale).round() as i32;
            let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, r, r);
            if rgn != 0 {
                SetWindowRgn(self.tip_hwnd, rgn, 1);
            }
        }
        SetWindowPos(self.tip_hwnd, -1, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        if size_changed || content_changed {
            InvalidateRect(self.tip_hwnd, std::ptr::null(), 0);
        }
        self.visible = true;
    }

    unsafe fn is_fullscreen(&self, fg: isize) -> bool {
        let style = GetWindowLongPtrW(fg, GWL_STYLE);
        if (style as u32) & WS_CAPTION != 0 {
            return false;
        }
        let mut wr: RECT = unsafe { std::mem::zeroed() };
        if GetWindowRect(fg, &mut wr) == 0 {
            return false;
        }
        let mut mi: MONITORINFO = unsafe { std::mem::zeroed() };
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST as u32);
        if mon == 0 || GetMonitorInfoW(mon, &mut mi) == 0 {
            return false;
        }
        let m = mi.rcMonitor;
        wr.left <= m.left + 2 && wr.top <= m.top + 2 && wr.right >= m.right - 2 && wr.bottom >= m.bottom - 2
    }

    unsafe fn poll(&mut self) {
        if self.paused {
            self.hide_tip();
            return;
        }
        let fg = GetForegroundWindow();
        if fg == 0 || fg == self.tip_hwnd || fg == self.msg_hwnd {
            self.hide_tip();
            return;
        }
        if IsIconic(fg) != 0 {
            self.hide_tip();
            return;
        }
        if IGNORE_FULLSCREEN && self.is_fullscreen(fg) {
            self.hide_tip();
            return;
        }
        let mut gti: GUITHREADINFO = unsafe { std::mem::zeroed() };
        gti.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        let tid = GetWindowThreadProcessId(fg, std::ptr::null_mut());
        let got = GetGUIThreadInfo(tid, &mut gti);
        let focus = if got != 0 && gti.hwndFocus != 0 {
            gti.hwndFocus
        } else {
            fg
        };
        let st = match detect_state(focus) {
            Some(s) => s,
            None => return,
        };
        let (is_cn, symbol, lang, _conv) = st;
        let caps = if is_cn { 0 } else { caps_lock() };
        let mut caret = caret_screen(&gti);
        let mut mouse = false;
        if caret.is_none() {
            if !self.follow_mouse {
                self.hide_tip();
                return;
            }
            let mut pt = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut pt) == 0 {
                return;
            }
            caret = Some((pt.x, pt.y, pt.y));
            mouse = true;
        }
        let caret = caret.unwrap();
        let key = (focus, is_cn, symbol, lang, caps);
        let changed = key != self.last_key;
        let t = now();
        if changed {
            self.last_key = key;
            self.show_tip(fg, caret, build_glyphs(is_cn, symbol, lang, caps));
            self.show_until = t + TIMEOUT;
        } else if self.visible {
            if t >= self.show_until {
                self.hide_tip();
            } else if mouse {
                self.show_tip(fg, caret, build_glyphs(is_cn, symbol, lang, caps));
            }
        }
    }

    unsafe fn tray_add(&mut self) {
        if self.tray_added {
            return;
        }
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.msg_hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = LoadIconW(0, IDI_APPLICATION);
        let tip = wstr("ImTipLite 输入法提示");
        let n = tip.len().min(128);
        nid.szTip[..n].copy_from_slice(&tip[..n]);
        if Shell_NotifyIconW(NIM_ADD, &nid) != 0 {
            self.tray_added = true;
        }
    }

    unsafe fn tray_del(&mut self) {
        if !self.tray_added {
            return;
        }
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.msg_hwnd;
        nid.uID = 1;
        Shell_NotifyIconW(NIM_DELETE, &nid);
        self.tray_added = false;
    }

    unsafe fn paint(&mut self, hwnd: isize) {
        let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
        let hdc = BeginPaint(hwnd, &mut ps);
        if hdc == 0 {
            return;
        }
        let w = self.draw_w;
        let h = self.draw_h;
        if w <= 0 || h <= 0 {
            EndPaint(hwnd, &ps);
            return;
        }
        // 复用内存 DC/位图(尺寸不变时不重建)
        if self.mem_dc == 0 || self.mem_w != w || self.mem_h != h {
            if self.mem_bmp != 0 {
                DeleteObject(self.mem_bmp as HGDIOBJ);
                self.mem_bmp = 0;
            }
            if self.mem_dc != 0 {
                DeleteDC(self.mem_dc);
                self.mem_dc = 0;
            }
            let screen = GetDC(0);
            self.mem_dc = CreateCompatibleDC(screen);
            self.mem_bmp = CreateCompatibleBitmap(screen, w, h);
            ReleaseDC(0, screen);
            self.mem_w = w;
            self.mem_h = h;
        }
        let md = self.mem_dc;
        let oldbmp = SelectObject(md, self.mem_bmp as HGDIOBJ);
        let rc = RECT { left: 0, top: 0, right: w, bottom: h };
        let brush = CreateSolidBrush(rgb(BG));
        FillRect(md, &rc, brush);
        DeleteObject(brush as HGDIOBJ);
        SetBkMode(md, TRANSPARENT as i32);
        let scale = self.dpi_scale();
        let pad = (4.0 * scale).round() as i32;
        let cell = (26.0 * scale).round() as i32;
        let oldf = SelectObject(md, self.font as HGDIOBJ);
        for (i, (text, color)) in self.draw_cells.iter().enumerate() {
            let mut cell_rc = RECT {
                left: pad + (i as i32) * cell,
                top: 0,
                right: pad + ((i as i32) + 1) * cell,
                bottom: h,
            };
            SetTextColor(md, rgb(*color));
            let tw = wstr(text);
            DrawTextW(
                md,
                tw.as_ptr(),
                -1,
                &mut cell_rc,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
        }
        SelectObject(md, oldf);
        BitBlt(hdc, 0, 0, w, h, md, 0, 0, SRCCOPY);
        SelectObject(md, oldbmp);
        EndPaint(hwnd, &ps);
    }
}

// 全局单例(单线程消息循环)+ 重入保护(防止 paint 在 poll 持有期间重入)
static mut APP: Option<App> = None;
thread_local! {
    static IN_CALL: Cell<bool> = const { Cell::new(false) };
}

// ---- 窗口过程 ----
unsafe extern "system" fn msg_wndproc(hwnd: isize, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wp == TIMER_POLL => {
            let reenter = IN_CALL.with(|f| f.replace(true));
            if !reenter {
                if let Some(a) = APP.as_mut() {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        a.poll();
                    }));
                }
            }
            IN_CALL.with(|f| f.set(false));
            0
        }
        WM_TRAY => {
            let m = lp as u32 & 0xFFFF;
            if m == WM_LBUTTONUP {
                if let Some(a) = APP.as_mut() {
                    a.paused = !a.paused;
                    a.hide_tip();
                }
                return 0;
            }
            if m == WM_RBUTTONUP {
                // 先快照状态(不持有跨 TrackPopupMenu)
                let (paused, follow_mouse, msg_hwnd) = if let Some(a) = APP.as_mut() {
                    (a.paused, a.follow_mouse, a.msg_hwnd)
                } else {
                    return 0;
                };
                let menu = CreatePopupMenu();
                let flag = |b: bool| if b { MF_CHECKED } else { 0 };
                let s1 = wstr(if paused { "恢复输入提示(&P)" } else { "暂停输入提示(&P)" });
                AppendMenuW(menu, MF_STRING | flag(paused), IDM_PAUSE, s1.as_ptr());
                let s2 = wstr("鼠标位置兜底(兼容, &M)");
                AppendMenuW(menu, MF_STRING | flag(follow_mouse), IDM_MOUSE, s2.as_ptr());
                AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                let s3 = wstr("退出(&X)");
                AppendMenuW(menu, MF_STRING, IDM_EXIT, s3.as_ptr());
                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);
                let cmd = TrackPopupMenu(
                    menu,
                    TPM_RIGHTBUTTON | TPM_RETURNCMD,
                    pt.x,
                    pt.y,
                    0,
                    msg_hwnd,
                    std::ptr::null(),
                );
                PostMessageW(msg_hwnd, WM_NULL, 0, 0);
                DestroyMenu(menu);
                if let Some(a) = APP.as_mut() {
                    match cmd as usize {
                        IDM_PAUSE => {
                            a.paused = !a.paused;
                            a.hide_tip();
                        }
                        IDM_MOUSE => a.follow_mouse = !a.follow_mouse,
                        IDM_EXIT => { DestroyWindow(a.msg_hwnd); }
                        _ => {}
                    }
                }
            }
            0
        }
        WM_DESTROY => {
            if let Some(a) = APP.as_mut() {
                a.tray_del();
                KillTimer(a.msg_hwnd, TIMER_POLL);
                if a.tip_hwnd != 0 {
                    DestroyWindow(a.tip_hwnd);
                    a.tip_hwnd = 0;
                }
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
            let reenter = IN_CALL.with(|f| f.replace(true));
            let r = if !reenter {
                if let Some(a) = APP.as_mut() {
                    a.paint(hwnd);
                }
                0
            } else {
                0
            };
            IN_CALL.with(|f| f.set(false));
            r
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn run() -> i32 {
    let hinst = GetModuleHandleW(std::ptr::null()) as HMODULE as isize;

    let name = wstr("ImTipLite_SingleInstance");
    let _m = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
    if _m == 0 {
        return 0;
    }
    if GetLastError() == ERROR_ALREADY_EXISTS {
        return 0;
    }

    APP = Some(App::new(hinst));

    let cls = wstr("ImTipLiteMsg");
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW_VREDRAW,
        lpfnWndProc: Some(msg_wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst as HINSTANCE,
        hIcon: 0,
        hCursor: LoadCursorW(0, IDC_ARROW),
        hbrBackground: 0,
        lpszMenuName: std::ptr::null(),
        lpszClassName: cls.as_ptr(),
        hIconSm: 0,
    };
    RegisterClassExW(&wc);
    let hwnd = CreateWindowExW(
        0,
        cls.as_ptr(),
        wstr("ImTipLite").as_ptr(),
        WS_POPUP,
        0, 0, 0, 0,
        0, 0, hinst as HINSTANCE, std::ptr::null(),
    );
    if hwnd == 0 {
        return 1;
    }
    if let Some(a) = APP.as_mut() {
        a.msg_hwnd = hwnd;
        a.tray_add();
    }
    SetTimer(hwnd, TIMER_POLL, POLL_MS, None);

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        let r = GetMessageW(&mut msg, 0, 0, 0);
        if r <= 0 {
            break;
        }
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    0
}

fn main() {
    unsafe {
        std::process::exit(run());
    }
}
