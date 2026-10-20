#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""ImTipLite —— 复刻 ImTip 的「输入法状态光标跟随提示」(仅输入法提示功能)

检测原理与规则完整复刻 aardio 标准库 key.ime.state:
    https://www.aardio.com/zh-cn/docs/library-guide/std/key/imeState.html
    https://www.aardio.com/zh-cn/docs/library-reference/key/ime.html

核心机制:
  1) 向目标线程的默认 IME 窗口发送 WM_IME_CONTROL(0x0283):
       IMC_GETOPENSTATUS   = 0x0005   取输入法开/关状态
       IMC_GETCONVERSIONMODE = 0x0001 取转换模式(中英/标点/全半角)
     (跨进程 ImmGetContext 不可用, 这是与 ImTip 相同的方案)
  2) GetKeyboardLayout(前台线程) 取键盘布局语言 ID, 与 opened/convMode
     按 key.ime.state 规则组合判定 中/英/中文标点/全角/半角/大写。
  3) GetGUIThreadInfo 取光标位置(rcCaret 相对 hwndCaret 客户区),
     ClientToScreen 转屏幕坐标, 提示条跟随显示在光标右下方。
  4) 鼠标位置兜底(ImTip 的退化行为)默认关闭: 检测不到光标就不显示,
     不会跟着鼠标乱跑。可用配置 follow_mouse=1 或托盘菜单开启。

仅用 Python 标准库(ctypes/winreg/configparser), 无任何第三方依赖。

用法:
    pythonw imtip_lite.py           后台运行(推荐)
    python imtip_lite.py            前台运行(便于查看日志)
    python imtip_lite.py --state    调试模式: 每秒打印一次当前输入法状态
"""
import ctypes
import os
import sys
import time
import traceback
import configparser
from ctypes import wintypes as wt

# ---------------------------------------------------------------------------
# 常量
# ---------------------------------------------------------------------------
WM_IME_CONTROL = 0x0283
IMC_GETCONVERSIONMODE = 0x0001
IMC_GETOPENSTATUS = 0x0005

SMTO_ABORTIFHUNG = 0x0002
GUI_CARETBLINKING = 0x00000001

WM_PAINT = 0x000F
WM_ERASEBKGND = 0x0014
WM_TIMER = 0x0113
WM_COMMAND = 0x0111
WM_DESTROY = 0x0002
WM_NULL = 0x0000
WM_QUIT = 0x0012
WM_APP = 0x8000
WM_TRAY = WM_APP + 1

WM_LBUTTONUP = 0x0202
WM_RBUTTONUP = 0x0205

WS_POPUP = 0x80000000
WS_EX_LAYERED = 0x00080000
WS_EX_TRANSPARENT = 0x00000020
WS_EX_TOOLWINDOW = 0x00000080
WS_EX_TOPMOST = 0x00000008
WS_EX_NOACTIVATE = 0x08000000

SWP_NOSIZE = 0x0001
SWP_NOACTIVATE = 0x0010
SWP_SHOWWINDOW = 0x0040
SWP_HIDEWINDOW = 0x0080
HWND_TOPMOST = -1
SW_HIDE = 0
SW_SHOWNOACTIVATE = 4

LWA_ALPHA = 0x00000002

VK_CAPITAL = 0x14

GWL_STYLE = -16
WS_CAPTION = 0x00C00000

MONITOR_DEFAULTTONEAREST = 0x00000002

NIM_ADD = 0x00000000
NIM_MODIFY = 0x00000001
NIM_DELETE = 0x00000002
NIF_MESSAGE = 0x00000001
NIF_ICON = 0x00000002
NIF_TIP = 0x00000004

MF_STRING = 0x00000000
MF_SEPARATOR = 0x00000800
MF_CHECKED = 0x00000008
MF_GRAYED = 0x00000001
TPM_RIGHTBUTTON = 0x0002
TPM_RETURNCMD = 0x0100

TRANSPARENT = 1
DT_CENTER = 0x00000001
DT_VCENTER = 0x00000004
DT_SINGLELINE = 0x00000020
FW_BOLD = 700
CLEARTYPE_QUALITY = 5
DEFAULT_CHARSET = 1

SRCCOPY = 0x00CC0020
DIB_RGB_COLORS = 0

ERROR_ALREADY_EXISTS = 183

IDM_PAUSE = 1001
IDM_MOUSE = 1002
IDM_AUTOSTART = 1003
IDM_RELOAD = 1004
IDM_EXIT = 1005
IDM_SHOW_ON_CHANGE = 1006

TIMER_POLL = 1

APP_NAME = "ImTipLite"

# ---------------------------------------------------------------------------
# WinAPI 载入
# ---------------------------------------------------------------------------
user32 = ctypes.WinDLL("user32", use_last_error=True)
gdi32 = ctypes.WinDLL("gdi32", use_last_error=True)
imm32 = ctypes.WinDLL("imm32", use_last_error=True)
shell32 = ctypes.WinDLL("shell32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

LRESULT = ctypes.c_ssize_t
WPARAM = ctypes.c_size_t
LPARAM = ctypes.c_ssize_t
HWND = ctypes.c_void_p
HICON = ctypes.c_void_p
HMENU = ctypes.c_void_p
HBRUSH = ctypes.c_void_p
HFONT = ctypes.c_void_p
HBITMAP = ctypes.c_void_p

WNDPROC = ctypes.WINFUNCTYPE(LRESULT, HWND, wt.UINT, WPARAM, LPARAM)


class RECT(ctypes.Structure):
    _fields_ = [("left", wt.LONG), ("top", wt.LONG),
                ("right", wt.LONG), ("bottom", wt.LONG)]


class POINT(ctypes.Structure):
    _fields_ = [("x", wt.LONG), ("y", wt.LONG)]


class GUITHREADINFO(ctypes.Structure):
    _fields_ = [("cbSize", wt.DWORD), ("flags", wt.DWORD),
                ("hwndActive", HWND), ("hwndFocus", HWND),
                ("hwndCapture", HWND), ("hwndMenuOwner", HWND),
                ("hwndMoveSize", HWND), ("hwndCaret", HWND),
                ("rcCaret", RECT)]


class MONITORINFO(ctypes.Structure):
    _fields_ = [("cbSize", wt.DWORD), ("rcMonitor", RECT),
                ("rcWork", RECT), ("dwFlags", wt.DWORD)]


class WNDCLASSEXW(ctypes.Structure):
    _fields_ = [("cbSize", wt.UINT), ("style", wt.UINT),
                ("lpfnWndProc", WNDPROC), ("cbClsExtra", ctypes.c_int),
                ("cbWndExtra", ctypes.c_int), ("hInstance", wt.HINSTANCE),
                ("hIcon", HICON), ("hCursor", wt.HANDLE),
                ("hbrBackground", HBRUSH), ("lpszMenuName", wt.LPCWSTR),
                ("lpszClassName", wt.LPCWSTR), ("hIconSm", HICON)]


class MSG(ctypes.Structure):
    _fields_ = [("hwnd", HWND), ("message", wt.UINT), ("wParam", WPARAM),
                ("lParam", LPARAM), ("time", wt.DWORD), ("pt", POINT)]


class BITMAPINFOHEADER(ctypes.Structure):
    _fields_ = [("biSize", wt.DWORD), ("biWidth", wt.LONG),
                ("biHeight", wt.LONG), ("biPlanes", wt.WORD),
                ("biBitCount", wt.WORD), ("biCompression", wt.DWORD),
                ("biSizeImage", wt.DWORD), ("biXPelsPerMeter", wt.LONG),
                ("biYPelsPerMeter", wt.LONG), ("biClrUsed", wt.DWORD),
                ("biClrImportant", wt.DWORD)]


class BITMAPINFO(ctypes.Structure):
    _fields_ = [("bmiHeader", BITMAPINFOHEADER), ("bmiColors", wt.DWORD * 1)]


class ICONINFO(ctypes.Structure):
    _fields_ = [("fIcon", wt.BOOL), ("xHotspot", wt.DWORD),
                ("yHotspot", wt.DWORD), ("hbmMask", HBITMAP),
                ("hbmColor", HBITMAP)]


class GUID(ctypes.Structure):
    _fields_ = [("Data1", wt.DWORD), ("Data2", wt.WORD),
                ("Data3", wt.WORD), ("Data4", wt.BYTE * 8)]


class _NID_UNION(ctypes.Union):
    _fields_ = [("uTimeout", wt.UINT), ("uVersion", wt.UINT)]


class NOTIFYICONDATAW(ctypes.Structure):
    _fields_ = [("cbSize", wt.DWORD), ("hWnd", HWND), ("uID", wt.UINT),
                ("uFlags", wt.UINT), ("uCallbackMessage", wt.UINT),
                ("hIcon", HICON), ("szTip", wt.WCHAR * 128),
                ("dwState", wt.DWORD), ("dwStateMask", wt.DWORD),
                ("szInfo", wt.WCHAR * 256), ("union", _NID_UNION),
                ("szInfoTitle", wt.WCHAR * 64), ("dwInfoFlags", wt.DWORD),
                ("guidItem", GUID), ("hBalloonIcon", HICON)]


# ---------------------------------------------------------------------------
# 函数原型(64 位下必须声明, 否则指针/返回值被截断)
# ---------------------------------------------------------------------------
_P = ctypes.POINTER


def _sig(fn, restype, *argtypes):
    fn.restype = restype
    fn.argtypes = list(argtypes)


_sig(user32.GetForegroundWindow, HWND)
_sig(user32.GetWindowThreadProcessId, wt.DWORD, HWND, _P(wt.DWORD))
_sig(user32.GetKeyboardLayout, wt.HANDLE, wt.DWORD)
_sig(user32.SendMessageTimeoutW, LRESULT, HWND, wt.UINT, WPARAM, LPARAM,
      wt.UINT, wt.UINT, _P(ctypes.c_size_t))
_sig(user32.DestroyMenu, wt.BOOL, HMENU)
_sig(user32.RegisterWindowMessageW, wt.UINT, wt.LPCWSTR)
_sig(user32.GetGUIThreadInfo, wt.BOOL, wt.DWORD, _P(GUITHREADINFO))
_sig(user32.ClientToScreen, wt.BOOL, HWND, _P(POINT))
_sig(user32.AttachThreadInput, wt.BOOL, wt.DWORD, wt.DWORD, wt.BOOL)
_sig(user32.GetKeyState, ctypes.c_short, ctypes.c_int)
_sig(user32.GetAsyncKeyState, ctypes.c_short, ctypes.c_int)
_sig(user32.GetCursorPos, wt.BOOL, _P(POINT))
_sig(user32.IsIconic, wt.BOOL, HWND)
_sig(user32.GetWindowLongW, wt.LONG, HWND, ctypes.c_int)
_sig(user32.GetWindowRect, wt.BOOL, HWND, _P(RECT))
_sig(user32.MonitorFromWindow, HWND, HWND, wt.DWORD)
_sig(user32.MonitorFromPoint, HWND, POINT, wt.DWORD)
_sig(user32.GetMonitorInfoW, wt.BOOL, HWND, _P(MONITORINFO))
_sig(user32.GetDpiForWindow, wt.UINT, HWND)
_sig(user32.SetLayeredWindowAttributes, wt.BOOL, HWND, wt.DWORD, wt.BYTE, wt.DWORD)
_sig(user32.SetWindowPos, wt.BOOL, HWND, HWND, ctypes.c_int,
      ctypes.c_int, ctypes.c_int, ctypes.c_int, wt.UINT)
_sig(user32.SetWindowRgn, ctypes.c_int, HWND, wt.HANDLE, wt.BOOL)
_sig(gdi32.CreateRoundRectRgn, wt.HANDLE, ctypes.c_int, ctypes.c_int,
      ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int)
_sig(user32.ShowWindow, wt.BOOL, HWND, ctypes.c_int)
_sig(user32.SetTimer, wt.LPARAM, HWND, wt.LPARAM, wt.UINT, ctypes.c_void_p)
_sig(user32.KillTimer, wt.BOOL, HWND, wt.LPARAM)
_sig(user32.DefWindowProcW, LRESULT, HWND, wt.UINT, WPARAM, LPARAM)
_sig(user32.PostQuitMessage, None, ctypes.c_int)
_sig(user32.PostMessageW, wt.BOOL, HWND, wt.UINT, WPARAM, LPARAM)
_sig(user32.RegisterClassExW, wt.UINT, _P(WNDCLASSEXW))
_sig(user32.CreateWindowExW, HWND, wt.DWORD, wt.LPCWSTR, wt.LPCWSTR,
      wt.DWORD, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int,
      HWND, HMENU, wt.HINSTANCE, wt.LPVOID)
_sig(user32.DestroyWindow, wt.BOOL, HWND)
_sig(user32.GetMessageW, wt.BOOL, _P(MSG), HWND, wt.UINT, wt.UINT)
_sig(user32.TranslateMessage, wt.BOOL, _P(MSG))
_sig(user32.DispatchMessageW, LRESULT, _P(MSG))
_sig(user32.LoadIconW, HICON, wt.HANDLE, wt.LPCWSTR)
_sig(user32.CreateIconIndirect, HICON, _P(ICONINFO))
_sig(user32.DestroyIcon, wt.BOOL, HICON)
_sig(user32.CreatePopupMenu, HMENU)
_sig(user32.AppendMenuW, wt.BOOL, HMENU, wt.UINT, wt.WPARAM, wt.LPCWSTR)
_sig(user32.TrackPopupMenu, wt.UINT, HMENU, wt.UINT, ctypes.c_int,
      ctypes.c_int, ctypes.c_int, HWND, _P(RECT))
_sig(user32.InvalidateRect, wt.BOOL, HWND, _P(RECT), wt.BOOL)
_sig(user32.UpdateWindow, wt.BOOL, HWND)
_sig(user32.GetDC, wt.HANDLE, HWND)
_sig(user32.ReleaseDC, ctypes.c_int, HWND, wt.HANDLE)
_sig(user32.BeginPaint, wt.HANDLE, HWND, wt.LPVOID)
_sig(user32.EndPaint, wt.BOOL, HWND, wt.LPVOID)
_sig(user32.FillRect, wt.INT, wt.HANDLE, _P(RECT), HBRUSH)
_sig(gdi32.SetBkMode, wt.INT, wt.HANDLE, ctypes.c_int)
_sig(gdi32.SetTextColor, wt.DWORD, wt.HANDLE, wt.DWORD)
_sig(gdi32.CreateFontW, HFONT, ctypes.c_int, ctypes.c_int, ctypes.c_int,
      ctypes.c_int, ctypes.c_int, wt.BOOL, wt.BOOL, wt.BOOL, wt.UINT,
      wt.UINT, wt.UINT, wt.UINT, wt.UINT, wt.LPCWSTR)
_sig(gdi32.CreateSolidBrush, HBRUSH, wt.DWORD)
_sig(user32.DrawTextW, ctypes.c_int, wt.HANDLE, wt.LPCWSTR, ctypes.c_int,
      _P(RECT), wt.UINT)

_sig(gdi32.CreateCompatibleDC, wt.HANDLE, wt.HANDLE)
_sig(gdi32.CreateCompatibleBitmap, HBITMAP, wt.HANDLE, ctypes.c_int, ctypes.c_int)
_sig(gdi32.SelectObject, wt.HANDLE, wt.HANDLE, wt.HANDLE)
_sig(gdi32.BitBlt, wt.BOOL, wt.HANDLE, ctypes.c_int, ctypes.c_int,
      ctypes.c_int, ctypes.c_int, wt.HANDLE, ctypes.c_int, ctypes.c_int, wt.DWORD)
_sig(gdi32.CreateDIBSection, HBITMAP, wt.HANDLE, _P(BITMAPINFO), wt.UINT,
      wt.LPVOID, wt.HANDLE, wt.DWORD)
_sig(gdi32.CreateBitmap, HBITMAP, ctypes.c_int, ctypes.c_int,
      wt.UINT, wt.UINT, wt.LPVOID)
_sig(gdi32.DeleteObject, wt.BOOL, wt.HANDLE)
_sig(gdi32.DeleteDC, wt.BOOL, wt.HANDLE)
_sig(gdi32.LineTo, wt.BOOL, wt.HANDLE, ctypes.c_int, ctypes.c_int)

_sig(imm32.ImmGetDefaultIMEWnd, HWND, HWND)

_sig(shell32.Shell_NotifyIconW, wt.BOOL, wt.DWORD, _P(NOTIFYICONDATAW))

_sig(kernel32.GetCurrentThreadId, wt.DWORD)
_sig(kernel32.CreateMutexW, wt.HANDLE, wt.LPVOID, wt.BOOL, wt.LPCWSTR)
_sig(kernel32.GetLastError, wt.DWORD)
_sig(kernel32.GetModuleHandleW, wt.HMODULE, wt.LPCWSTR)

if hasattr(user32, "SetProcessDpiAwarenessContext"):
    _sig(user32.SetProcessDpiAwarenessContext, wt.BOOL, wt.HANDLE)
if hasattr(user32, "SetProcessDPIAware"):
    _sig(user32.SetProcessDPIAware, wt.BOOL)

# ---------------------------------------------------------------------------
# 输入法状态检测(与 aardio key.ime.state 规则一致)
# ---------------------------------------------------------------------------
# key.ime.state 中 conversionLangIds 映射的 CJK 键盘布局语言 ID
CJK_LANG_IDS = {0x0804, 0x0404, 0x0C04, 0x1404, 0x0412, 0x0411}

# 非 CJK 布局的显示字符(仿 ImTip tipChars)
LANG_CHARS = {
    0x0409: "En", 0x0809: "En", 0x0C09: "En", 0x4009: "En",
    0x0407: "DE", 0x0807: "DE",
    0x040C: "FR", 0x080C: "FR", 0x0C0C: "FR",
    0x0410: "IT", 0x0810: "IT",
    0x040A: "ES", 0x080A: "ES", 0x0C0A: "ES",
    0x0416: "PT", 0x0816: "PT",
    0x0419: "RU",
    0x0415: "PL",
    0x041A: "HR",
    0x0413: "NL",
    0x0406: "DA",
    0x040B: "FI",
    0x0414: "NB",
    0x0405: "CS",
    0x040E: "HU",
    0x041B: "SK",
    0x0424: "SL",
    0x0402: "BG",
    0x041F: "TR",
    0x0401: "AR",
    0x040D: "HE",
    0x0804: "Zh", 0x0404: "Zh", 0x0C04: "Zh", 0x1404: "Zh",
    0x0411: "Ja", 0x0412: "Ko",
}


def _ime_control(hwnd, cmd):
    """向 hwnd 所在线程的默认 IME 窗口发送 WM_IME_CONTROL。返回值或 None(失败)。"""
    ime_wnd = imm32.ImmGetDefaultIMEWnd(HWND(hwnd))
    if not ime_wnd:
        return None
    res = ctypes.c_size_t(0)
    ok = user32.SendMessageTimeoutW(
        HWND(ime_wnd), WM_IME_CONTROL, WPARAM(cmd), LPARAM(0),
        SMTO_ABORTIFHUNG, 200, ctypes.byref(res))
    if not ok:
        return None
    return res.value


def get_lang_id(hwnd):
    """取窗口所在线程键盘布局的语言 ID(LOWORD(HKL))。"""
    tid = user32.GetWindowThreadProcessId(HWND(hwnd), None)
    hkl = user32.GetKeyboardLayout(tid)
    return hkl & 0xFFFF


def get_caps_lock():
    """取 CapsLock 切换状态(0/1)。优先附着前台线程输入队列读取。"""
    fg = user32.GetForegroundWindow()
    if fg:
        tid = user32.GetWindowThreadProcessId(fg, None)
        my_tid = kernel32.GetCurrentThreadId()
        if tid and tid != my_tid and user32.AttachThreadInput(my_tid, tid, True):
            try:
                return user32.GetKeyState(VK_CAPITAL) & 1
            finally:
                user32.AttachThreadInput(my_tid, tid, False)
    return user32.GetAsyncKeyState(VK_CAPITAL) & 1


def detect_state(focus_hwnd):
    """检测输入法状态。返回 (is_cn, symbol_mode, lang_id, conv_mode) 或 None(未知)。

    symbol_mode:
        0 已关闭输入转换(英文键盘)
        1 英文半角标点
        2 英文全角标点
        3 中文标点
    完整复刻 aardio key.ime.state 的判定逻辑。
    """
    opened = _ime_control(focus_hwnd, IMC_GETOPENSTATUS)
    conv = _ime_control(focus_hwnd, IMC_GETCONVERSIONMODE)

    if opened is None and conv is None:
        return None  # 目标无响应/无法访问(如管理员窗口), 状态未知

    lang_id = get_lang_id(focus_hwnd)

    if opened is None:
        opened = 0
    if opened and lang_id == 0x0409:
        opened = 0  # 布局是英文键盘却报打开 → 视为英文
    if conv is None:
        return (False, None, lang_id, None)

    symbol_mode = 1
    if (not opened) and conv in (0, 1) and lang_id not in CJK_LANG_IDS:
        symbol_mode = 0
    elif (conv & 0x400) and opened:  # IME_CMODE_SYMBOL 中文标点
        symbol_mode = 3
    elif conv & 0x0008:  # IME_CMODE_FULLSHAPE 全角
        symbol_mode = 2
    elif conv & 0x0100:  # IME_CMODE_NOCONVERSION 强制英文
        opened = 0

    is_cn = bool(opened) and bool(conv & 0x0003)  # IME_CMODE_LANGUAGE
    return (is_cn, symbol_mode, lang_id, conv)


def get_caret_screen(gti):
    """从 GUITHREADINFO 取光标屏幕坐标。返回 (x, caret底y, caret顶y) 或 None。"""
    if not gti.hwndCaret or not (gti.flags & GUI_CARETBLINKING):
        return None
    bottom = POINT(gti.rcCaret.left, gti.rcCaret.bottom)
    top = POINT(0, gti.rcCaret.top)
    if not user32.ClientToScreen(HWND(gti.hwndCaret), ctypes.byref(bottom)):
        return None
    user32.ClientToScreen(HWND(gti.hwndCaret), ctypes.byref(top))
    return (bottom.x, bottom.y, top.y)


# ---------------------------------------------------------------------------
# 配置
# ---------------------------------------------------------------------------
DEFAULT_CONFIG = """\
; ImTipLite 配置文件(修改后 1 秒内自动生效)
[general]
; 检测间隔(毫秒), ImTip 默认 50
poll_interval = 50
; 提示显示时长(秒), ImTip 默认 2
timeout = 2.0
; 1 = 仅在切换输入目标或状态变化时显示; 0 = 光标存在就一直显示
show_on_change = 1
; 1 = 允许鼠标位置兜底(检测不到光标时跟随鼠标, 即 ImTip 的退化行为)
follow_mouse = 0
; 1 = 前台窗口为全屏(游戏/视频)时不显示
ignore_fullscreen = 1
; 提示条相对光标的偏移(不受 DPI 影响的逻辑像素)
offset_x = 14
offset_y = 2

[appearance]
; 窗口整体透明度 0-255
alpha = 235
; 圆角半径
radius = 9
; 单元格宽度 / 窗口高度
cell = 26
height = 30
; 字体名
font = Microsoft YaHei UI
; 颜色 RRGGBB
bg = 262B33
color_cn = 6BE585
color_en = 6BB6FF
color_symbol = FFB454
color_caps = FFD66B
color_half = 7A8291
"""


def config_path():
    d = os.path.join(os.environ.get("APPDATA", os.path.expanduser("~")),
                     "ImTipLite")
    os.makedirs(d, exist_ok=True)
    return os.path.join(d, "config.ini")


class Settings:
    def __init__(self, path=None):
        self.path = path or config_path()
        self.mtime = 0.0
        self.cfg = None
        self.reload(force=True)

    def reload(self, force=False):
        try:
            if not force and os.path.getmtime(self.path) == self.mtime:
                return False
        except OSError:
            pass
        try:
            with open(self.path, "r", encoding="utf-8") as f:
                text = f.read()
        except OSError:
            text = ""
        if "[general]" not in text:
            try:
                with open(self.path, "w", encoding="utf-8") as f:
                    f.write(DEFAULT_CONFIG)
            except OSError:
                pass
        self.cfg = configparser.ConfigParser()
        self.cfg.read_string(DEFAULT_CONFIG)
        try:
            self.cfg.read(self.path, encoding="utf-8")
        except (configparser.Error, OSError):
            pass
        try:
            self.mtime = os.path.getmtime(self.path)
        except OSError:
            self.mtime = 0.0
        return True

    def _g(self, section, opt, fallback):
        try:
            return self.cfg.get(section, opt, fallback=fallback)
        except (configparser.Error, KeyError):
            return fallback

    def _gi(self, section, opt, fallback):
        try:
            return self.cfg.getint(section, opt, fallback=fallback)
        except ValueError:
            return fallback

    def _gf(self, section, opt, fallback):
        try:
            return self.cfg.getfloat(section, opt, fallback=fallback)
        except ValueError:
            return fallback

    def _color(self, section, opt, fallback):
        s = self._g(section, opt, fallback)
        try:
            return tuple(int(s[i:i + 2], 16) for i in (0, 2, 4))
        except (ValueError, TypeError):
            return fallback

    @property
    def poll_interval(self): return max(10, self._gi("general", "poll_interval", 50))
    @property
    def timeout(self): return max(0.2, self._gf("general", "timeout", 2.0))
    @property
    def show_on_change(self): return bool(self._gi("general", "show_on_change", 1))
    @property
    def follow_mouse(self): return bool(self._gi("general", "follow_mouse", 0))
    @property
    def ignore_fullscreen(self): return bool(self._gi("general", "ignore_fullscreen", 1))
    @property
    def offset_x(self): return self._gf("general", "offset_x", 14.0)
    @property
    def offset_y(self): return self._gf("general", "offset_y", 2.0)

    @property
    def alpha(self): return max(0, min(255, self._gi("appearance", "alpha", 235)))
    @property
    def radius(self): return max(0, self._gi("appearance", "radius", 9))
    @property
    def cell(self): return max(10, self._gi("appearance", "cell", 26))
    @property
    def height(self): return max(10, self._gi("appearance", "height", 30))
    @property
    def font_name(self): return self._g("appearance", "font", "Microsoft YaHei UI")
    @property
    def bg(self): return self._color("appearance", "bg", (0x26, 0x2B, 0x33))
    @property
    def color_cn(self): return self._color("appearance", "color_cn", (0x6B, 0xE5, 0x85))
    @property
    def color_en(self): return self._color("appearance", "color_en", (0x6B, 0xB6, 0xFF))
    @property
    def color_symbol(self): return self._color("appearance", "color_symbol", (0xFF, 0xB4, 0x54))
    @property
    def color_caps(self): return self._color("appearance", "color_caps", (0xFF, 0xD6, 0x6B))
    @property
    def color_half(self): return self._color("appearance", "color_half", (0x7A, 0x82, 0x91))

    def set_flag(self, section, opt, value):
        try:
            self.cfg.set(section, opt, "1" if value else "0")
            with open(self.path, "w", encoding="utf-8") as f:
                self.cfg.write(f)
            self.mtime = os.path.getmtime(self.path)
        except OSError:
            pass


def _rgb(c):
    """(r,g,b) → COLORREF"""
    return c[0] | (c[1] << 8) | (c[2] << 16)


# ---------------------------------------------------------------------------
# 提示窗内容计算
# ---------------------------------------------------------------------------
def build_glyphs(is_cn, symbol_mode, lang_id, caps, settings):
    """返回 [(text, color_rgb), ...] 最多两个单元格。"""
    # 第 1 格: 中英/语言
    if lang_id in CJK_LANG_IDS:
        t1 = "中" if is_cn else "英"
        c1 = settings.color_cn if is_cn else settings.color_en
    else:
        t1 = LANG_CHARS.get(lang_id, "")
        c1 = settings.color_en
    cells = []
    if t1:
        cells.append((t1, c1))

    # 第 2 格: 大写/标点状态(优先级: 大写 > 中文标点 > 全角 > 半角)
    t2, c2 = "", None
    if caps and not is_cn:
        t2, c2 = "A", settings.color_caps
    elif symbol_mode == 3:
        t2, c2 = "。", settings.color_symbol
    elif symbol_mode == 2:
        t2, c2 = "●", settings.color_symbol
    elif symbol_mode == 1:
        t2, c2 = "·", settings.color_half
    if t2:
        cells.append((t2, c2))
    return cells


# ---------------------------------------------------------------------------
# 应用主体
# ---------------------------------------------------------------------------
class ImTipLite:
    def __init__(self):
        self.settings = Settings()
        self.msg_hwnd = None
        self.tip_hwnd = None
        self.visible = False
        self.show_until = 0.0
        self.last_key = None
        self.paused = False
        self.autostart = self._check_autostart()
        self.tray_icon = None
        self.tray_added = False
        self.font = None
        self.font_px = 0
        self.draw_cells = []
        self.draw_w = 0
        self.draw_h = 0
        self.tip_w = 0
        self.tip_h = 0
        self.last_reload_check = 0.0
        self._msg_proc_ref = WNDPROC(self._msg_wndproc)
        self._tip_proc_ref = WNDPROC(self._tip_wndproc)
        self._taskbar_msg = user32.RegisterWindowMessageW("TaskbarCreated")
        # DPI 感知
        try:
            if not user32.SetProcessDpiAwarenessContext(wt.HANDLE(-4)):
                user32.SetProcessDPIAware()
        except (AttributeError, OSError):
            try:
                user32.SetProcessDPIAware()
            except (AttributeError, OSError):
                pass

    # ---- 日志 ----
    def log(self, msg):
        try:
            p = os.path.join(os.path.dirname(self.settings.path), "error.log")
            with open(p, "a", encoding="utf-8") as f:
                f.write("%s %s\n" % (time.strftime("%Y-%m-%d %H:%M:%S"), msg))
        except OSError:
            pass

    # ---- 自启动 ----
    def _autostart_cmd(self):
        import winreg
        if getattr(sys, "frozen", False):
            return '"%s"' % sys.executable
        exe = os.path.join(os.path.dirname(sys.executable), "pythonw.exe")
        if not os.path.exists(exe):
            exe = sys.executable
        return '"%s" "%s"' % (exe, os.path.abspath(__file__))

    def _autostart_key(self):
        import winreg
        return winreg.OpenKey(
            winreg.HKEY_CURRENT_USER,
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            0, winreg.KEY_SET_VALUE | winreg.KEY_QUERY_VALUE)

    def _check_autostart(self):
        try:
            import winreg
            with self._autostart_key() as k:
                winreg.QueryValueEx(k, APP_NAME)
            return True
        except OSError:
            return False

    def _set_autostart(self, on):
        try:
            import winreg
            with self._autostart_key() as k:
                if on:
                    winreg.SetValueEx(k, APP_NAME, 0, winreg.REG_SZ,
                                      self._autostart_cmd())
                else:
                    winreg.DeleteValue(k, APP_NAME)
            return True
        except OSError:
            return False

    # ---- 窗口 ----
    def _register_class(self, name, proc, style_extra=0):
        wc = WNDCLASSEXW()
        wc.cbSize = ctypes.sizeof(WNDCLASSEXW)
        wc.style = style_extra
        wc.lpfnWndProc = proc
        wc.hInstance = kernel32.GetModuleHandleW(None)
        wc.lpszClassName = name
        user32.RegisterClassExW(ctypes.byref(wc))

    def _get_scale(self, hwnd):
        try:
            dpi = user32.GetDpiForWindow(HWND(hwnd))
            if dpi:
                return dpi / 96.0
        except (AttributeError, OSError):
            pass
        return 1.0

    # ---- 托盘图标(程序绘制: 深色圆角方块 + 中/英字) ----
    def _make_tray_icon(self, is_cn=True):
        size = 32
        hdc = user32.GetDC(None)
        bmi = BITMAPINFO()
        bmi.bmiHeader.biSize = ctypes.sizeof(BITMAPINFOHEADER)
        bmi.bmiHeader.biWidth = size
        bmi.bmiHeader.biHeight = -size  # 自顶向下
        bmi.bmiHeader.biPlanes = 1
        bmi.bmiHeader.biBitCount = 32
        bmi.bmiHeader.biCompression = 0
        bits = ctypes.c_void_p()
        hbmp = gdi32.CreateDIBSection(
            hdc, ctypes.byref(bmi), DIB_RGB_COLORS, ctypes.byref(bits), None, 0)
        if not hbmp:
            user32.ReleaseDC(None, hdc)
            return None
        buf = ctypes.cast(bits, ctypes.POINTER(ctypes.c_ubyte))
        # 初始化透明
        for i in range(size * size):
            buf[i * 4 + 3] = 0
        # 绘制: 圆角方块
        r = 8
        bg = (0x2E, 0x6B, 0xE6) if is_cn else (0x3A, 0x40, 0x4C)
        fg = (0xFF, 0xFF, 0xFF)
        for y in range(size):
            for x in range(size):
                # 圆角判定
                cx = min(max(x, r), size - 1 - r)
                cy = min(max(y, r), size - 1 - r)
                dx, dy = x - cx, y - cy
                if dx * dx + dy * dy > r * r:
                    continue
                o = (y * size + x) * 4
                buf[o + 0] = bg[2]
                buf[o + 1] = bg[1]
                buf[o + 2] = bg[0]
                buf[o + 3] = 255
        # 画字
        md = gdi32.CreateCompatibleDC(hdc)
        old = gdi32.SelectObject(md, hbmp)
        font = gdi32.CreateFontW(-19, 0, 0, 0, FW_BOLD, 0, 0, 0, DEFAULT_CHARSET,
                                  0, 0, 0, 0, self.settings.font_name)
        oldf = gdi32.SelectObject(md, font)
        gdi32.SetBkMode(md, TRANSPARENT)
        gdi32.SetTextColor(md, _rgb(fg))
        rc = RECT(0, 0, size, size)
        user32.DrawTextW(md, "中" if is_cn else "A", -1, ctypes.byref(rc),
                         DT_CENTER | DT_VCENTER | DT_SINGLELINE)
        gdi32.SelectObject(md, oldf)
        gdi32.SelectObject(md, old)
        gdi32.DeleteObject(font)
        gdi32.DeleteDC(md)
        mask = gdi32.CreateBitmap(size, size, 1, 1, None)
        ii = ICONINFO(True, 0, 0, mask, hbmp)
        hicon = user32.CreateIconIndirect(ctypes.byref(ii))
        gdi32.DeleteObject(hbmp)
        gdi32.DeleteObject(mask)
        user32.ReleaseDC(None, hdc)
        return hicon

    def _nid(self, flags):
        nid = NOTIFYICONDATAW()
        nid.cbSize = ctypes.sizeof(NOTIFYICONDATAW)
        nid.hWnd = self.msg_hwnd
        nid.uID = 1
        nid.uFlags = flags
        return nid

    def _tray_add(self):
        if self.tray_icon is None:
            self.tray_icon = self._make_tray_icon()
        nid = self._nid(NIF_MESSAGE | NIF_ICON | NIF_TIP)
        nid.uCallbackMessage = WM_TRAY
        nid.hIcon = self.tray_icon
        nid.szTip = "ImTipLite 输入法状态提示"
        if shell32.Shell_NotifyIconW(NIM_ADD, ctypes.byref(nid)):
            self.tray_added = True

    def _tray_del(self):
        if self.tray_added:
            shell32.Shell_NotifyIconW(NIM_DELETE, ctypes.byref(self._nid(0)))
            self.tray_added = False

    # ---- 托盘菜单 ----
    def _show_menu(self):
        menu = user32.CreatePopupMenu()
        user32.AppendMenuW(menu, MF_STRING | (MF_CHECKED if self.paused else 0),
                           IDM_PAUSE, "暂停输入提示(&P)")
        user32.AppendMenuW(menu, MF_STRING | (MF_CHECKED if not self.paused else 0),
                           IDM_SHOW_ON_CHANGE,
                           "仅切换输入目标或状态后显示(&S)")
        user32.AppendMenuW(menu, MF_STRING | (MF_CHECKED if self.settings.follow_mouse else 0),
                           IDM_MOUSE, "鼠标位置兜底(兼容模式, &M)")
        user32.AppendMenuW(menu, MF_STRING | MF_SEPARATOR, 0, None)
        user32.AppendMenuW(menu, MF_STRING | (MF_CHECKED if self.autostart else 0),
                           IDM_AUTOSTART, "开机自启(&R)")
        user32.AppendMenuW(menu, MF_STRING, IDM_RELOAD, "重载配置(&L)")
        user32.AppendMenuW(menu, MF_STRING | MF_SEPARATOR, 0, None)
        user32.AppendMenuW(menu, MF_STRING, IDM_EXIT, "退出(&X)")
        pt = POINT()
        user32.GetCursorPos(ctypes.byref(pt))
        cmd = user32.TrackPopupMenu(
            menu, TPM_RIGHTBUTTON | TPM_RETURNCMD, pt.x, pt.y,
            0, self.msg_hwnd, None)
        user32.PostMessageW(self.msg_hwnd, WM_NULL, 0, 0)
        user32.DestroyMenu(menu)
        return cmd

    def _on_tray(self, msg):
        if msg == WM_LBUTTONUP:
            self._toggle_pause()
        elif msg == WM_RBUTTONUP:
            cmd = self._show_menu()
            if cmd == IDM_PAUSE:
                self._toggle_pause()
            elif cmd == IDM_MOUSE:
                self.settings.set_flag("general", "follow_mouse",
                                       not self.settings.follow_mouse)
                self.settings.reload(force=True)
            elif cmd == IDM_AUTOSTART:
                if self._set_autostart(not self.autostart):
                    self.autostart = not self.autostart
            elif cmd == IDM_RELOAD:
                self.settings.reload(force=True)
            elif cmd == IDM_EXIT:
                user32.DestroyWindow(self.msg_hwnd)

    def _toggle_pause(self):
        self.paused = not self.paused
        self.hide_tip()

    # ---- 提示窗 ----
    def _ensure_tip(self):
        if self.tip_hwnd:
            return
        self._register_class("ImTipLiteTip", self._tip_proc_ref, 0x0002)  # CS_VREDRAW|CS_HREDRAW
        ex = (WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST |
              WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)
        self.tip_hwnd = user32.CreateWindowExW(
            ex, "ImTipLiteTip", "", WS_POPUP,
            0, 0, 10, 10, None, None, kernel32.GetModuleHandleW(None), None)
        if self.tip_hwnd:
            user32.SetLayeredWindowAttributes(
                self.tip_hwnd, 0, self.settings.alpha, LWA_ALPHA)

    def _tip_wndproc(self, hwnd, msg, wp, lp):
        try:
            if msg == WM_ERASEBKGND:
                return 1
            if msg == WM_PAINT:
                self._paint(hwnd)
                return 0
        except Exception:
            self.log(traceback.format_exc())
        return user32.DefWindowProcW(hwnd, msg, wp, lp)

    def _paint(self, hwnd):
        class PAINTSTRUCT(ctypes.Structure):
            _fields_ = [("hdc", wt.HANDLE), ("fErase", wt.BOOL),
                        ("rcPaint", RECT), ("fRestore", wt.BOOL),
                        ("fIncUpdate", wt.BOOL), ("rgbReserved", wt.BYTE * 32)]
        ps = PAINTSTRUCT()
        hdc = user32.BeginPaint(hwnd, ctypes.byref(ps))
        w, h = self.draw_w, self.draw_h
        if w <= 0 or h <= 0:
            user32.EndPaint(hwnd, ctypes.byref(ps))
            return
        md = gdi32.CreateCompatibleDC(hdc)
        bmp = gdi32.CreateCompatibleBitmap(hdc, w, h)
        old = gdi32.SelectObject(md, bmp)
        rc = RECT(0, 0, w, h)
        brush = gdi32.CreateSolidBrush(_rgb(self.settings.bg))
        user32.FillRect(md, ctypes.byref(rc), brush)
        gdi32.DeleteObject(brush)
        gdi32.SetBkMode(md, TRANSPARENT)
        scale = self._scale
        pad = int(round(4 * scale))
        cell = int(round(self.settings.cell * scale))
        for i, (text, color) in enumerate(self.draw_cells):
            cell_rc = RECT(pad + i * cell, 0, pad + (i + 1) * cell, h)
            gdi32.SetTextColor(md, _rgb(color))
            user32.DrawTextW(md, text, -1, ctypes.byref(cell_rc),
                             DT_CENTER | DT_VCENTER | DT_SINGLELINE)
        gdi32.BitBlt(hdc, 0, 0, w, h, md, 0, 0, SRCCOPY)
        gdi32.SelectObject(md, old)
        gdi32.DeleteObject(bmp)
        gdi32.DeleteDC(md)
        user32.EndPaint(hwnd, ctypes.byref(ps))

    def _show_tip(self, fg_hwnd, caret, cells):
        """caret: (x, caret底y, caret顶y)"""
        self._ensure_tip()
        if not self.tip_hwnd:
            return
        scale = self._get_scale(fg_hwnd)
        self._scale = scale
        pad = int(round(4 * scale))
        cell = int(round(self.settings.cell * scale))
        h = int(round(self.settings.height * scale))
        w = pad * 2 + cell * max(1, len(cells))
        font_px = int(round(17 * scale))
        if font_px != self.font_px or not self.font:
            if self.font:
                gdi32.DeleteObject(self.font)
            self.font = gdi32.CreateFontW(
                -font_px, 0, 0, 0, FW_BOLD, 0, 0, 0, DEFAULT_CHARSET,
                0, 0, 0, 0, self.settings.font_name)
            self.font_px = font_px

        x = caret[0] + int(round(self.settings.offset_x * scale))
        y = caret[1] + int(round(self.settings.offset_y * scale))
        mi = MONITORINFO()
        mi.cbSize = ctypes.sizeof(MONITORINFO)
        pt = POINT(caret[0], caret[1])
        mon = user32.MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST)
        if mon and user32.GetMonitorInfoW(mon, ctypes.byref(mi)):
            work = mi.rcWork
            if y + h > work.bottom:
                y = caret[2] - h - int(round(self.settings.offset_y * scale))
            if x + w > work.right:
                x = work.right - w - 4
            if x < work.left:
                x = work.left
            if y < work.top:
                y = work.top

        self.draw_cells = cells
        self.draw_w, self.draw_h = w, h
        r = int(round(self.settings.radius * scale))
        rg = gdi32.CreateRoundRectRgn(0, 0, w + 1, h + 1, r, r)
        if rg:
            user32.SetWindowRgn(self.tip_hwnd, rg, True)
        user32.SetLayeredWindowAttributes(
            self.tip_hwnd, 0, self.settings.alpha, LWA_ALPHA)
        user32.SetWindowPos(
            self.tip_hwnd, HWND_TOPMOST, x, y, w, h,
            SWP_NOACTIVATE | SWP_SHOWWINDOW)
        user32.InvalidateRect(self.tip_hwnd, None, True)
        self.visible = True

    def hide_tip(self):
        if self.visible and self.tip_hwnd:
            user32.ShowWindow(self.tip_hwnd, SW_HIDE)
        self.visible = False

    # ---- 状态轮询 ----
    def is_fullscreen(self, fg):
        try:
            style = user32.GetWindowLongW(HWND(fg), GWL_STYLE)
        except OSError:
            return False
        if style & WS_CAPTION:
            return False
        wr = RECT()
        mi = MONITORINFO()
        mi.cbSize = ctypes.sizeof(MONITORINFO)
        if not user32.GetWindowRect(HWND(fg), ctypes.byref(wr)):
            return False
        mon = user32.MonitorFromWindow(HWND(fg), MONITOR_DEFAULTTONEAREST)
        if not mon or not user32.GetMonitorInfoW(mon, ctypes.byref(mi)):
            return False
        m = mi.rcMonitor
        return (wr.left <= m.left + 2 and wr.top <= m.top + 2 and
                wr.right >= m.right - 2 and wr.bottom >= m.bottom - 2)

    def poll(self):
        # 配置热更新
        now = time.monotonic()
        if now - self.last_reload_check > 1.0:
            self.last_reload_check = now
            if self.settings.reload():
                if self.tip_hwnd:
                    user32.SetLayeredWindowAttributes(
                        self.tip_hwnd, 0, self.settings.alpha, LWA_ALPHA)
                if self._timer_interval != self.settings.poll_interval:
                    self._timer_interval = self.settings.poll_interval
                    user32.KillTimer(self.msg_hwnd, TIMER_POLL)
                    user32.SetTimer(self.msg_hwnd, TIMER_POLL,
                                    self._timer_interval, None)

        if self.paused:
            self.hide_tip()
            return

        fg = user32.GetForegroundWindow()
        if not fg or fg == self.tip_hwnd or fg == self.msg_hwnd:
            self.hide_tip()
            self.last_key = None
            return
        if user32.IsIconic(fg):
            self.hide_tip()
            return
        if self.settings.ignore_fullscreen and self.is_fullscreen(fg):
            self.hide_tip()
            return

        gti = GUITHREADINFO()
        gti.cbSize = ctypes.sizeof(GUITHREADINFO)
        focus = None
        if user32.GetGUIThreadInfo(
                user32.GetWindowThreadProcessId(fg, None), ctypes.byref(gti)):
            focus = gti.hwndFocus or gti.hwndActive
        if not focus:
            focus = fg

        st = detect_state(focus)
        if st is None:
            return  # 目标无响应, 保持现状
        is_cn, symbol_mode, lang_id, conv = st
        caps = get_caps_lock() if not is_cn else False

        caret = get_caret_screen(gti)
        mouse_fallback = False
        if caret is None:
            if not self.settings.follow_mouse:
                self.hide_tip()
                return
            pt = POINT()
            if not user32.GetCursorPos(ctypes.byref(pt)):
                return
            caret = (pt.x, pt.y, pt.y)
            mouse_fallback = True

        key = (focus, is_cn, symbol_mode, lang_id, caps)
        changed = (key != self.last_key)

        if self.settings.show_on_change:
            if changed:
                self.last_key = key
                self._show_tip(fg, caret,
                               build_glyphs(is_cn, symbol_mode, lang_id,
                                            caps, self.settings))
                self.show_until = now + self.settings.timeout
            else:
                if self.visible and now >= self.show_until:
                    self.hide_tip()
        else:
            # 一直显示模式
            if changed or not self.visible:
                self.last_key = key
                if mouse_fallback:
                    # 鼠标模式跟随鼠标移动
                    self._show_tip(fg, caret,
                                   build_glyphs(is_cn, symbol_mode, lang_id,
                                                caps, self.settings))
                elif not self.visible:
                    self._show_tip(fg, caret,
                                   build_glyphs(is_cn, symbol_mode, lang_id,
                                                caps, self.settings))

    # ---- 主窗口 ----
    def _msg_wndproc(self, hwnd, msg, wp, lp):
        try:
            if msg == WM_TIMER and wp == TIMER_POLL:
                try:
                    self.poll()
                except Exception:
                    self.log(traceback.format_exc())
                return 0
            if msg == WM_TRAY:
                try:
                    self._on_tray(lp & 0xFFFF)
                except Exception:
                    self.log(traceback.format_exc())
                return 0
            if msg == self._taskbar_msg:
                self.tray_added = False
                self._tray_add()
                return 0
            if msg == WM_DESTROY:
                self._tray_del()
                user32.KillTimer(hwnd, TIMER_POLL)
                self.hide_tip()
                if self.tip_hwnd:
                    user32.DestroyWindow(self.tip_hwnd)
                    self.tip_hwnd = None
                user32.PostQuitMessage(0)
                return 0
        except Exception:
            self.log(traceback.format_exc())
        return user32.DefWindowProcW(hwnd, msg, wp, lp)

    def run(self):
        self._register_class("ImTipLiteMsg", self._msg_proc_ref, 0x0002)
        self.msg_hwnd = user32.CreateWindowExW(
            0, "ImTipLiteMsg", APP_NAME, WS_POPUP,
            0, 0, 0, 0, None, None, kernel32.GetModuleHandleW(None), None)
        if not self.msg_hwnd:
            self.log("create msg window failed: %s" % ctypes.get_last_error())
            return 1
        self._tray_add()
        self._timer_interval = self.settings.poll_interval
        user32.SetTimer(self.msg_hwnd, TIMER_POLL,
                        self._timer_interval, None)
        ms = MSG()
        while user32.GetMessageW(ctypes.byref(ms), None, 0, 0):
            user32.TranslateMessage(ctypes.byref(ms))
            user32.DispatchMessageW(ctypes.byref(ms))
        return 0


# ---------------------------------------------------------------------------
# 调试模式: --state
# ---------------------------------------------------------------------------
def debug_state_loop():
    print("ImTipLite 状态检测调试(每秒一次, Ctrl+C 退出)")
    while True:
        fg = user32.GetForegroundWindow()
        gti = GUITHREADINFO()
        gti.cbSize = ctypes.sizeof(GUITHREADINFO)
        focus = None
        ok = False
        if fg:
            ok = user32.GetGUIThreadInfo(
                user32.GetWindowThreadProcessId(fg, None), ctypes.byref(gti))
            focus = gti.hwndFocus or gti.hwndActive or fg
        st = detect_state(focus) if focus else None
        caret = get_caret_screen(gti) if ok else None
        caps = get_caps_lock()
        print("fg=0x%x focus=%s state=%s caret=%s caps=%s" % (
            fg or 0,
            ("0x%x" % focus) if focus else "?",
            st, caret, caps), flush=True)
        time.sleep(1.0)


def main():
    if "--state" in sys.argv:
        try:
            debug_state_loop()
        except KeyboardInterrupt:
            pass
        return 0

    mutex = kernel32.CreateMutexW(None, False, "ImTipLite_SingleInstance")
    if kernel32.GetLastError() == ERROR_ALREADY_EXISTS:
        return 0

    app = ImTipLite()
    return app.run()


if __name__ == "__main__":
    sys.exit(main())
