# 开发环境（Windows 11 x64）

本文描述在 Windows 上构建 ohMyWork（Rust + GPUI Kit，V1）所需的最小原生依赖、
诊断命令与缺失组件的安装步骤。约束见 `AGENTS.md` 与
`docs/adr/adr-0001-windows-rust-gpui.md`。

## 依赖范围

按 GPUI Kit 的 Windows 安装说明，本仓库只需要以下 Windows 原生依赖：

- Windows 10 或更高版本（V1 目标为 Windows 11 x64）。
- Visual Studio **Build Tools 或 Community**，安装 **“使用 C++ 的桌面开发”**
  工作负载，其中包含 MSVC 编译器与 Windows SDK。
- Rust 工具链（见下方固定版本），目标为 MSVC（非 GNU/MinGW）。
- 构建所用终端可执行 `cmake --version`。

不照抄构建整个 Zed 所需的 PostgreSQL、wasmtime 等无关设置。不需要
Electron/WebView。

## 固定工具链

`rust-toolchain.toml` 固定 Rust `1.98.1`、`x86_64-pc-windows-msvc` target，
组件仅 `rustfmt`、`clippy`。`rustup` 在此仓库内执行 `cargo`/`rustc` 时会自动
使用该工具链；目标 toolchain 未安装时，会提示按 rustup 自动安装流程执行（显式
执行前视为未授权）。

`.cargo/config.toml` 仅做仓库级配置：固定默认 target 为 MSVC，不覆盖
linker（rustc 自动从注册表定位 VS 的 link.exe）。该文件不修改用户或全局 Cargo
配置、不修改 PATH。

### 动态快捷键 API 边界

GPUI Kit 0.8.1 配套的 `KeyBinding::new` 只接受 `&'static str`，用于静态绑定。
`keymap.jsonc` 的动态键位在转换 `ctrl+s` 等写法后，使用 `KeyBinding::load` 注册；
上下文谓词按 `KeyContext` 解析，命令复用 `CommandId::to_action()`。
注册失败会在标准错误输出中报告上下文、键位和命令，不泄漏字符串来延长生命周期。

### 编辑器 Rope API 边界

`editor::SourceRope` 使用 GPUI Kit 0.7.0 组件导出的 Ropey 2.0.0-beta.1，
与解析器使用的 Ropey 1.6.1 不同。组件 Rope 的字节长度为 `len()`；
字节与字符索引转换使用 `byte_to_char_idx()` / `char_to_byte_idx()`，
不能混用解析器 Rope 的 `len_bytes()` / `byte_to_char()` / `char_to_byte()`。

`EditorState::set_value` 接受可转换为 `InputContent` 的文本，不接受 Rope。
初始化、文档同步和外部重载在此边界使用 `to_string()`；源码历史仍保留 Rope。
`SourceSession` 仅供 `documents` 及其子模块使用，不向 crate 根重导出。

### Markdown 正文绘制与验证边界

`MarkdownElement` 使用 GPUI Kit 0.8.1 的公开 `editor_host` 块渲染接口。
`request_layout` 返回的状态持有同一个正文 `AnyElement`，随后将预绘制和绘制
分别委托给它；不能只请求布局后丢弃子元素，否则文件已读入也会出现空白正文。
叶几何通过 `collect_leaf_geometry` 在预绘制期间采集，每帧替换，不使用固定行高或虚构坐标。
外部版本变化先清除旧投影，再为当前 snapshot 重建，避免清除新投影后停留在 Loading。

当前源投影、表格网格编辑和原生输入仍未完成，不能据此宣称同区编辑已验收。
`cargo check` 仅验证类型兼容性；显示检查必须使用当前源码构建的新程序。
`--editor-demo` 只检查内置样例显示，真实文件加载仍须在工作台打开 Markdown 检查；
两者都不能替代保存、任务局部写回或外部修改同步的验收。


## 运行诊断

在任意目录执行（脚本按自身位置定位仓库根，不依赖当前工作目录）：

```powershell
pwsh -NoProfile -File tools/doctor.ps1
```

输出每项为 `OK / WARN / ERROR / INFO`，并给出发现来源与版本；不打印凭据，
不修改任何环境。退出码可机器判断：

| 退出码 | 含义 |
|---|---|
| 0 | 全部 OK |
| 1 | 存在 WARN（可构建但有注意项，如 MSVC 未载入当前 shell） |
| 2 | 存在 ERROR（缺必需组件，构建条件不满足） |

CI 可加 `-Json` 只输出一个 JSON 对象（含 `exit` 与逐项状态）；`-SkipCompile`
跳过 MSVC 空程序编译冒烟测试（例如无 MSVC 的静态检查任务）。

## 普通 shell 与 VS Developer shell

- **普通 shell（非开发者终端）**：`rustup`/`cargo`/`cmake` 在 PATH 时可正常
  检测；`cl.exe` 通常不在 PATH，doctor 会报告 **“MSVC 已安装但未载入当前
  终端”**（WARN），而不是“未安装”。此时链路、SDK、工具链检查仍完整。
- **VS Developer shell**：先载入开发环境再运行 doctor，MSVC 空程序编译
  应显示 OK：

  ```bat
  call "C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
  pwsh -NoProfile -File tools\doctor.ps1
  ```

  也可从开始菜单使用 “x64 Native Tools Command Prompt for VS 2026” 或
  “Developer PowerShell for VS 2026”。GPUI/cc 构建需要链接时，请在载入了
  MSVC 的终端中执行 `cargo` 命令。

## doctor 的只读保证

rustup ≥ 1.29 会在多数子命令（如 `rustup toolchain list`）中对
rust-toolchain.toml 指定的缺失工具链**自动安装**（deprecated 行为，参考
<https://github.com/rust-lang/rustup/issues/4836>）。因此 doctor **绝不调用
rustup**，只读取 rustup home 下的文件：工具链目录名、`lib\rustlib\components`
组件清单、`lib\rustlib\<target>\` 目标目录；`rustc --version` 等用工具链目录内
的 `bin\rustc.exe` 直接执行。这样 doctor 在任何情况下都不会引入安装副作用。

## 安装缺失组件（先经用户允许，再执行）

doctor 只检测并给出命令，不自行安装；任何安装前需用户明确授权。

> rustup ≥ 1.29 下，在仓库内手动运行
> `rustup toolchain install …`、`rustup target add …`、以及**任何**会解析
> active toolchain 的 rustup 命令都是“安装”动作；普通 `cargo`/`rustc` 同理
> 会按需安装缺失工具链。请仅在确认需要时执行。

- **缺固定工具链（rustup 已装）**（替换 `1.98.1` 为固定版本；此命令**会**安装）：

- **缺固定工具链（rustup 已装）**：

  ```powershell
  rustup toolchain install 1.98.1 --profile minimal --component rustfmt,clippy
  rustup target add x86_64-pc-windows-msvc --toolchain 1.98.1
  ```

- **缺 rustup / Rust**：用 rustup-init 安装（`winget install Rustlang.Rustup`
  或官网安装器），完成后重开终端。
- **缺 CMake**：

  Visual Studio Build Tools 的“C++ CMake tools for Windows”组件在 Developer
  shell（vcvars64.bat 载入后）中已提供 `cmake`，例如本机
  `C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\Common7\IDE\
  CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe`（4.3.1-msvc1）。
  GPUI 构建在 Developer shell 中进行时通常无需额外安装；普通 shell 中想全局
  使用 cmake 时：

  ```powershell
  winget install Kitware.CMake
  ```

  或在 VS Installer 中为现有实例勾选 **“C++ CMake tools for Windows”** 组件，
  完成后重开终端。CMake 已安装但不在 PATH 时，doctor 会给出实际路径并提示
  手动调用或自行将目录加入用户 PATH。
- **缺 MSVC / Windows SDK**：VS Installer 中修改安装，勾选
  **“使用 C++ 的桌面开发”** 工作负载（含 MSVC 与 Windows SDK）。

安装后重新运行 `tools/doctor.ps1` 直到退出码 0（普通 shell 允许仅剩
“MSVC 未载入”类 WARN，在 Developer shell 中应为 0）。

## 验证命令（与脚本参数一致）

后续里程碑的常规验证（需在 Developer shell 中执行链接相关命令）：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -p ohmywork --bin ohmywork --locked
cargo run -p ohmywork --bin ohmywork --locked -- --editor-demo
```

默认入口在 Workspace 初始化或恢复后显示真实工作台：文件树、Markdown 编辑、搜索、任务、Homepage 与 Pi 面板。`--editor-demo` 只用于隔离检查编辑器 fixture，不能作为真实文件保存或工作台集成的验收证据。

图片/PDF 预览需要同目录的 `omw-preview-worker.exe`，PDF 还需要发布资产中的 `pdfium.dll`。性能脚本 `pwsh -File tools/bench.ps1 -CorpusDir <path> -Smoke` 只测索引与搜索 API，不测 GUI 启动、输入到显示或十标签内存预算。

CI（GitHub Actions `windows-latest`）已预装 VS、Windows SDK 与 CMake，无需
本文所述的人工步骤；CI 中先运行 `pwsh -File tools/doctor.ps1 -Json` 可在
步骤开头断言环境。

## 本机状态

本机的实际安装情况、版本与缺口以各里程碑的完成报告为准；doctor 输出是唯一
权威来源，不在此处记录静态快照。

**V1 发布验证**：详见 [修复验证记录](verification.md)与[发布检查表](release-checklist.md)。本轮包含真实原生工作台交互及后端性能测量；未测项目单独列出，不将测试通过等同于发布批准。严格格式 / Clippy / CI、真实 Pi、IME / Narrator / 多缩放、安装升级卸载及 GUI 性能预算仍须完成验收。