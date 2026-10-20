# ImTipLite (Rust 原生版) —— 方案 A 的原生核心

复刻 [ImTip](https://github.com/aardio/ImTip) 输入法状态光标提示的 **Rust 原生实现**。这是「方案 A」(Tauri 设置界面 + Rust 原生提示条)中的原生核心部分,独立可运行。

## 已实现

- 输入法状态检测:`WM_IME_CONTROL` + `IMC_GETOPENSTATUS/IMC_GETCONVERSIONMODE`,完整复刻 aardio `key.ime.state` 规则
- 光标跟随:`GetGUIThreadInfo.rcCaret` + `ClientToScreen`
- 原生 Win32 layered 提示窗 + GDI 绘制(中/英、中文标点。/全角●/半角·、大写 A)
- 托盘图标 + 右键菜单(暂停、鼠标兜底、退出)/ 左键暂停切换
- 单实例、全屏窗口忽略、DPI 缩放
- 鼠标兜底默认关闭(检测不到光标不显示,不跟着鼠标乱跑)

## 运行

```
imtip-rs.exe            直接运行(已构建好,见 target\release\)
```

或从源码构建(见下)。运行后:
- 托盘出现一个图标(系统默认 i 图标)
- 左键托盘 = 暂停/恢复
- 右键托盘 = 菜单

## 构建环境(本机已装好)

```
Rust 工具链         stable-x86_64-pc-windows-gnu
MinGW(WinLibs UCRT) BrechtSanders.WinLibs.POSIX.UCRT.LLVM
```

构建需要把 MinGW 的 bin 加进 PATH(供 rustc 的 dlltool/链接器使用):

```bash
export PATH="$HOME/.cargo/bin:/c/.../mingw64/bin:$PATH"
cargo build --release
```

产物 `target/release/imtip-rs.exe` 约 **870 KB**,只依赖系统 DLL(imm32/user32/gdi32/shell32/kernel32 + UCRT),无需安装任何运行库,Win10/11 可直接分发。

## 与 Python 版对比

| | Python+ctypes | Rust 原生(本版) |
|---|---|---|
| 体积 | PyInstaller ~10MB | 870KB 单文件 |
| 依赖 | 需 Python 或打包 | 无 |
| 提示条 | 原生 Win32 | 原生 Win32 |
| 检测逻辑 | 同 | 同(逐行移植) |

## 下一步:叠 Tauri 设置界面(方案 A 完整版)

原生提示条与托盘已就绪,接下来用 Tauri 2 包一层 WebView 窗口做:
- 外观/颜色可视化编辑(复刻 ImTip 调色)
- 轮询间隔、超时、偏移等配置 GUI
- 写入配置文件,原生核心热加载

原生核心通过 Tauri 的 Rust 后端直接调用(已是 Rust,无需 FFI 桥),提示条逻辑可整体复用 `imtip-rs` 的代码。
