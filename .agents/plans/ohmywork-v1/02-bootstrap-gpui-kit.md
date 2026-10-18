# 02 — 创建最小 Cargo/GPUI Kit 应用

## 执行边界与前置检查

本 Agent 不拥有里程碑01的对话。先读 `AGENTS.md`、`docs/adr/adr-0001-windows-rust-gpui.md`，以及 GPUI Kit 的 [Getting Started](https://gpui-kit.com/zh-CN/docs/getting-started.md)、[Windows安装](https://gpui-kit.com/zh-CN/docs/installation.md)、[测试](https://gpui-kit.com/zh-CN/docs/test.md) 和 [无障碍](https://gpui-kit.com/zh-CN/docs/accessibility.md)。再运行 `tools/doctor.ps1`；固定工具链、MSVC/SDK或CMake缺失时停止。

只搭建可编译工作区、Kit原生窗口、标准输入/按钮、无障碍样例和CI；不实现Markdown编辑器、Workspace或业务UI。

## 固定依赖决策

- Rust `1.98.1`、Windows MSVC。
- 根workspace精确锁定 `gpui-kit = "=0.7.0"` 并提交 `Cargo.lock`。生产使用默认 `component`、`assets` features；测试构建额外启用 `test-support`。
- 该Kit版本配套固定 `gpui-pre-* =0.3.7`。应用不得直接声明、patch或覆盖 `gpui`、`gpui_platform`、`gpui-component`、`gpui-base`，统一从 `gpui_kit` 重导出访问。
- 不启用未使用的tree-sitter、shell、webview等feature；不用Electron/WebView。

## 产物

- 根 `Cargo.toml`、`Cargo.lock`，初始成员 `crates/omw-app`。
- `crates/omw-app/src/main.rs` 及最少模块。
- 一个使用生产视图的 `#[gpui_kit::test]` UI集成测试。
- `.github/workflows/check.yml`，固定Windows runner并输出Rust/MSVC/SDK/CMake版本。
- 第三方许可证清单起点；不复制不明确许可的示例或Zed应用代码。

## 实现顺序

1. 建最小workspace和edition 2024 package，集中声明精确Kit依赖，生成lockfile；核对lockfile中的Kit 0.7.0与整套gpui-pre 0.3.7一致。
2. 用 `gpui_kit::application().with_assets(gpui_kit::assets::Assets)` 创建应用；在run闭包最早调用一次 `gpui_kit::init(cx)`。
3. 用 `gpui_kit::open_window(...)` 打开窗口，让Kit自动挂载 `Root`；不要再手动包第二个Root。
4. View长期持有 `Entity<InputState>`，渲染Kit `Input`与`Button`；输入状态不得在每次render重建。按钮更新同一份模型并展示结果。
5. 为Input设置稳定ID和可访问名称，验证Button名称、焦点、键盘激活、AccessKit状态；中文IME和Emoji使用Kit现成输入路径。
6. 测试使用 `gpui-kit` 的 `test-support`、`TestWindowExt` 驱动真实输入/点击并同时断言渲染状态和模型。人工IME/Narrator仍单列。
7. CI执行fmt、clippy、test、locked build。固定组合失败时保存完整错误，提出版本修订；禁止暗中改用宽泛版本、裸GPUI或WebView。

## 验证

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked`
- `cargo run -p omw-app`：中文组合、Emoji、Tab/Shift+Tab、Enter/Space按钮激活、焦点、Narrator基本动作、窗口关闭。
- 检查普通release依赖未启用 `test-support` 或无关feature。

## 完成定义与交接契约

后续可依赖：workspace可locked构建；应用入口、Assets、Kit init、Root、标准Input/Button及UI测试路径已证明可用。底层自定义Element能力留到05。不要提前创建空业务crate。完成报告列依赖解析、feature树、文件、命令结果、人工验证与跳过项。