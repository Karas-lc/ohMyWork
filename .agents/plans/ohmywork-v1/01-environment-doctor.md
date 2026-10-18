# 01 — 固定开发环境并实现 doctor

## 执行边界

这是全新上下文中的第一个里程碑。只建立可复现的 Windows Rust 构建环境和诊断脚本；不创建产品 Cargo 工程，不安装或修改 Pi，不实现 UI。

先读：`AGENTS.md`、`docs/adr/adr-0001-windows-rust-gpui.md` 和 GPUI Kit [Windows安装说明](https://gpui-kit.com/zh-CN/docs/installation.md)。检查仓库当前状态，不假定 Rust、CMake或VS未安装，也不覆盖用户环境。

## 目标与产物

- `rust-toolchain.toml`：固定 Rust `1.98.1`，MSVC target，组件仅 `rustfmt`、`clippy`。
- `.cargo/config.toml`：仅仓库级 Windows target/linker 配置；不改全局 PATH/config。
- `tools/doctor.ps1`：只读检测Rust、CMake、VS/MSVC、Windows SDK、目标与关键环境变量，返回可机器判断的退出码。
- `docs/development.md`：普通 shell、VS Developer shell、安装缺失组件和运行验证的准确步骤。

## 必须实现

1. 先检查标准与常见非标准 Rust/CMake/VS 安装位置；区分“已安装但当前 shell 未载入/不在PATH”与“未安装”。
2. doctor 输出每项 `OK/WARN/ERROR`、发现来源和版本，不输出凭据或修改环境。
3. 可定位 `cl.exe`、`link.exe`、Windows SDK和可执行的 `cmake --version`；在开发环境已加载时编译一个临时MSVC空程序，并清理临时文件。
4. 检查Rust版本、`x86_64-pc-windows-msvc` target、fmt/clippy；缺失时给出命令，不自行安装。任何安装必须另获用户明确授权。
5. 文档说明GPUI Kit只要求本步列出的Windows原生依赖；不照抄构建整个Zed所需的PostgreSQL/wasmtime等无关设置。
6. PowerShell 脚本开启严格错误处理，对空格/中文路径安全，不依赖当前工作目录。

## 验证

- 在普通 shell 运行 `pwsh -File tools/doctor.ps1`，能准确报告“MSVC未载入”而非“未安装”。
- 在已载入开发环境的 shell 中再次运行，MSVC 空程序编译成功。
- Rust、CMake或组件缺失通过可注入PATH或测试辅助函数验证分类和非零退出码。
- 文档命令与脚本参数一致；脚本不会修改用户/系统 PATH、Pi 或全局 Cargo 配置。

## 完成定义与交接契约

提交上述文件及验证结果。后续里程碑只可依赖：固定工具链声明存在；doctor 能判断本机是否可构建。若Rust 1.98.1、CMake或所需MSVC/SDK无法获得，停止并报告具体缺口，不改版本、不开始里程碑02。

完成报告只列：改动文件、检测到的版本/来源、运行命令及结果、未满足项。