# V1 发布检查表

更新于 2026-10-18。此前 Plan 28 的“所有计划和测试均通过”不是有效发布证据，已撤回；现行实测记录见 `docs/verification.md`。

## 当前已验证

- [x] `cargo test --workspace --locked`：266 tests、34 suites 通过。
- [x] `cargo build --locked -p omw-app -p omw-preview-worker --bins`：生产二进制构建通过，仍有警告。
- [x] 原生工作台：文件树和搜索打开真实 Markdown，保存前不写正文，Ctrl+S 写盘。
- [x] 原生关闭脏标签：保存 / 放弃 / 取消；外部覆盖丢弃脏正文及撤销历史。
- [x] 原生任务组件：只提交任务字段，保留其他脏正文，撤销不回退已提交字段。
- [x] 原生 PNG / PDF：真实 worker 位图、真实 PDFium 解码、PDF 翻页和缩放。
- [x] 原生 Homepage：实际指针拖动、键盘移动及调整尺寸、缩放、组件与布局持久化、分区目录入口。
- [x] Dock 标签恢复：真实路径重新读取磁盘，不恢复未保存草稿。
- [x] Pi RPC fixture：实际 cwd、中文草稿、Extension confirm 回复、abort 和有序退出；不是模型提供商验收。
- [x] 性能 runner 改为真实索引 / 查询 API；旧启动、输入和内存代理数字作废，不标为 GUI 预算通过。

## 发布门禁

按顺序执行并记录实际退出码，不以本文件中的勾选代替本次候选版本结果：

1. `pwsh -NoProfile -File tools/doctor.ps1`：确认工具链、MSVC 和原生依赖。此前普通终端报告 MSVC 未载入；成功链接不等于 Developer shell 配置已验证。
2. `cargo fmt --all -- --check`：仓库级检查；本轮只格式化修复模块，不能推断整个仓库干净。
3. `cargo clippy --workspace --locked --all-targets -- -D warnings`：严格门禁。当前存在未清理警告，不放宽规则，不声称通过。
4. `cargo test --workspace --locked`：全量自动回归，并补做下方人工场景。
5. `cargo build --release --locked --target x86_64-pc-windows-msvc`：生产 release 构建。按 manifest 获取并校验 PDFium；本机 debug 目录中的临时 DLL 不作为发布证据。
6. `pwsh -File tools/bench.ps1 -CorpusDir <独立语料目录> -Full`：记录后端原始样本；另测真实 GUI 启动到交互、搜索首批、输入到显示和十标签内存，不使用后端耗时替代。
7. 构建 NSIS 候选安装包；核对 `packaging/windows/assets-manifest.json`、第三方声明、打包文件及校验。
8. `pwsh -File tools/smoke-install.ps1`，并实际验证安装、手动升级、卸载、占用文件、回滚和 Workspace 保留。此前只发现安装目录不存在，不能当安装测试通过。

## 尚未完成的发布验收

- [ ] 真实中文输入法组合输入、Narrator、完整键盘导航、Windows 多种缩放比例。
- [ ] 用户安装的官方 Pi、真实模型 / 认证、会话恢复和不同用户 Extension；不得为验证擅自安装、升级或改配置。
- [ ] 原生文件选择器人工交互；预览进程故障后的人工恢复。
- [ ] 大语料 release 下的产品 GUI 性能预算，worker / Pi / GPU 独立资源测量。
- [ ] NSIS 候选包、签名、真实安装 / 升级 / 卸载与中文、空格路径。
- [ ] 严格格式、Clippy 和 CI 全绿。

这轮修复不是 V1 发布批准；未测项目保持未测，不沿用旧计划的通过结论。
