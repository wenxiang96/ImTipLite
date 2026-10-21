# ImTipLite

复刻 [ImTip](https://github.com/aardio/ImTip) 的「输入法状态光标跟随提示」。
Tauri 2(设置界面) + Rust 原生 Win32/GDI+ 提示条。单实例运行。

[![build](https://github.com/wenxiang96/ImTipLite/actions/workflows/build.yml/badge.svg)](https://github.com/wenxiang96/ImTipLite/actions/workflows/build.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## 直接使用(绿色版)

从 [Releases](https://github.com/wenxiang96/ImTipLite/releases) 下载 zip, 解压即用:

```
ImTipLite-vX.Y.Z-green.zip
├── ImTipLite.exe
└── WebView2Loader.dll   (必须与 exe 同目录)
```

**完全绿色便携**: 个人设置 `config.json` 保存 exe 同目录(不放 %APPDATA%、不写注册表),
拷贝整个目录到任意位置/U 盘即带走全部状态。旧版本升级时自动把 %APPDATA% 里的
配置迁移到 exe 目录, 不会丢设置。

- **托盘左键单击** → 打开设置
- **托盘右键** → 菜单(暂停 / 鼠标位置兜底 / 忽略全屏 / 打开设置 / 退出)
- **关闭设置窗口只是隐藏**,提示继续常驻;再次双击 `ImTipLite.exe`(或单击托盘)即可重新打开设置
- 重复双击 exe 不会多开实例,只会把已运行实例的设置窗口唤到前面
- **开机自启为静默启动**(注册表命令带 `--silent`):开机后只运行提示核心,不弹设置窗口
- 真正退出请用托盘右键 → 退出 ImTipLite

### 静默启动

设置界面开启"开机自启"后,注册表 `HKCU\...\Run` 写入 `"ImTipLite.exe" --silent`,
开机登录后只显示托盘图标和提示条,设置窗口保持隐藏。需要打开设置时:
双击 exe、或单击托盘图标即可。手动双击 exe(不带参数)启动时设置窗口正常显示。

## 功能

- 在光标处显示输入法状态:**中 / 英**、**中文标点 。/ 全角 ● / 半角 ·**、**大写 A**、多语言布局(日文/韩文/其他)
- **Shift** 切换中英、**CapsLock** 切换大小写 → 立即弹出提示,停留时长可调(0.5–10 秒)
- 无光标的窗口(浏览器 / 桌面 / 终端)按键触发时在鼠标位置显示
- 设置界面可调:行为、显示时长、检测间隔、偏移、字号、单元格宽、高度、圆角、边框、透明度、字体
- **字体为下拉选择**,选项实时枚举系统已安装字体(含用户级安装字体,自动过滤 @ 竖排变体)
- 7 种颜色(背景/边框/中文/英文/标点/大写/半角)+ 7 个图标字符均可自定义
- **透明背景**选项(只显示图标)
- 修改实时生效并自动保存到 exe 同目录 `config.json`(便携; exe 目录只读时回退 `%APPDATA%\ImTipLite`)

## 检测原理(与 ImTip / aardio `key.ime.state` 一致)

1. 向目标线程的默认 IME 窗口发送 `WM_IME_CONTROL`:
   - `IMC_GETOPENSTATUS (0x0005)` 输入法开关
   - `IMC_GETCONVERSIONMODE (0x0001)` 转换模式
   (跨进程无法用 `ImmGetContext`,只能发消息)
2. 判定规则:`conv & 1` 中日韩模式、`conv & 0x400` 中文标点、`conv & 8` 全角、`conv & 0x100` 强制英文、`langId == 0x409` 视为英文
3. 光标定位:`GetGUIThreadInfo.rcCaret`(相对 hwndCaret 客户区)+ `ClientToScreen`
4. 取不到光标时可选「鼠标位置兜底」

## 渲染

- 原生 layered 窗口 + `UpdateLayeredWindow` 逐像素 alpha(WS_EX_NOACTIVATE / TRANSPARENT / TOOLWINDOW)
- **4× 超采样**渲染后高质量缩小(HighQualityBicubic),圆角与文字边缘平滑
- GDI+ 绘制圆角(GDI+ 的 `FillPath` 在本环境恒返回 `ObjectBusy`,改用 16 段/角的抗锯齿多边形)
- 仅在尺寸/内容变化时重渲染,纯移动只 `SetWindowPos`
- **文字视觉居中**:先 `GdipMeasureString` 量排版尺寸,再在透明画布上实绘并扫描 alpha 取得**真实字形墨迹包围盒**,
  把墨迹中心对齐到单元格中心。中/英/。/·/A/En 全部走同一算法,无任何按字符的硬编码偏移。
  文字宽度超出单元格时(如 `char_other = "En"`)自动等比缩小字体,避免 GDI+ 截断。

## 图标

EXE 图标由 `tools/make_icon.py` 生成(蓝色圆角「中」)。≤128px 的 ICO 帧必须用 **BMP/DIB** 编码,
只有 256px 才可用 PNG 帧 —— 否则 Windows 资源管理器 / 任务栏会显示空白图标。
重新生成:

```bash
python tools/make_icon.py                 # 默认 teal(翡翠绿→青)
python tools/make_icon.py --preset blue   # 其他配色: blue / violet / amber
python tools/make_icon.py --preset teal --glyph 英
```

配色改动后需重新编译;`build.rs` 已为 `icons/` 声明 `rerun-if-changed`,
否则 cargo 不会重跑资源编译器,PE 里会残留旧图标。托盘图标取同一组渐变色
(`tip.rs` 的 `ICON_C1` / `ICON_C2`,与脚本的 `PRESETS` 保持一致)。

## 自检

```bash
cd tip-tauri/src-tauri
cargo run --release --bin tip-selftest     # 文字非空 + 水平/垂直居中(7 种状态组合)
cd ../.. && powershell -File tools/e2e_check.ps1   # 单实例 / 设置窗口 / Shift+Caps 触发 / 截图
```

`tip-selftest` 会断言每种字形都有可见像素且墨迹中心与单元格中心偏差 ≤1px,失败时退出码为 1。
`e2e_check.ps1` 会真实启动 `ImTipLite.exe`,模拟按键并保存 `_shot_*.bmp` 供核对。

## 目录

```
tip-tauri/            源码(Tauri 2 + Rust)
  src/                设置界面(HTML/CSS/JS)
  src-tauri/src/
    tip.rs            原生提示核心(检测/光标/渲染/托盘/单实例)
    lib.rs            Tauri 启动、配置持久化、命令
    bin/tip-selftest.rs  提示条渲染自检
  src-tauri/build.rs  tauri-build 资源(含 ICO 嵌入)
tools/make_icon.py    生成统一图标(ICO 内≤128px 用 BMP 帧)
tools/e2e_check.ps1   端到端运行时验证脚本
imtip-rs/             早期纯 Rust 无界面版源码
imtip_lite.py         早期 Python+ctypes 版
legacy-native/        早期纯 Rust 版二进制(留档)
```

## 从源码构建

需 Rust(GNU 工具链)+ MinGW + Node:

```bash
cd tip-tauri
npm install
npm run tauri build
```

或直接编译 Rust 后端:

```bash
export PATH="$HOME/.cargo/bin:/c/.../mingw64/bin:$PATH"
cd tip-tauri/src-tauri
cargo build --release
```

## 调试

设置环境变量 `IMTIP_DEBUG=1` 可输出日志到 `%TEMP%\imtip_dbg.log`;
`IMTIP_DUMP=1` 会把提示条位图导出为 `%TEMP%\imtip_dump.bmp`(超采样层为 `imtip_ss.bmp`)。
