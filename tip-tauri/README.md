# ImTipLite (方案 A 完整版) —— Tauri 设置界面 + Rust 原生提示条

复刻 [ImTip](https://github.com/aardio/ImTip) 的输入法状态光标提示功能。架构 = 方案 A:

- **Rust 原生提示条**(工作线程,原生 Win32 layered 窗口 + GDI)
- **Tauri 2 WebView 设置界面**(开关/参数可视化调节,实时生效)

## 目录

```
tip-tauri/
├── src/                      前端(纯 HTML/JS/CSS,无框架)
│   ├── index.html            设置页
│   ├── main.js               调 Tauri 命令读写配置
│   └── styles.css
└── src-tauri/
    ├── src/
    │   ├── lib.rs            Tauri 启动 + 命令 + 启动提示线程
    │   ├── tip.rs            原生提示核心(检测+光标跟随+提示窗+托盘)
    │   └── main.rs
    ├── Cargo.toml
    └── tauri.conf.json
```

## 运行/构建

需已配好工具链(Rust GNU + WinLibs MinGW,见上级 README)。

```bash
cd tip-tauri
npm install
npm run tauri dev      # 开发热运行
npm run tauri build   # 产出安装包/单文件
```

或直接跑已编译的 `src-tauri/target/debug/tip-tauri.exe`。

## 设置界面功能

- 开关:暂停、鼠标兜底、忽略全屏、仅切换时显示
- 参数:显示时长、检测间隔、水平/垂直偏移、透明度
- 恢复默认

改动 **实时生效**(写入共享 `Arc<Mutex<Config>>`,提示线程下一帧读取)。

关闭设置窗口时 **隐藏而非退出**(保持提示常驻);右键托盘「打开设置」可唤回。

## 检测原理(与 ImTip/aardio 一致)

- `WM_IME_CONTROL`(0x0283)→ `IMC_GETOPENSTATUS`(5)/`IMC_GETCONVERSIONMODE`(1),经默认 IME 窗口跨进程取状态
- `convMode` 位:`&1` 中文、`&0x400` 中文标点、`&8` 全角、`&0x100` 强英、`langId==0x409` 视英文
- 光标:`GetGUIThreadInfo.rcCaret`(相对 hwndCaret)→ `ClientToScreen`
- 鼠标兜底默认关闭(检测不到光标不显示,不跟着鼠标跑)

## 性能

- 检测在工作线程跑独立消息循环,**不阻塞** Tauri 主线程/UI
- 不用 `AttachThreadInput`(避免前台应用卡顿),CapsLock 用 `GetAsyncKeyState`
- `SendMessageTimeout` 60ms 超时,提示窗内存 DC/位图复用,纯移动不重绘
