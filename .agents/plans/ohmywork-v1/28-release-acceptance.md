# 28 — 执行 V1 发布验收与文档收口

## 执行边界与前置检查

先读 `AGENTS.md`、完整 `docs/product-requirements.md`、`docs/architecture.md`、现行ADR索引及所有Accepted ADR。确认01–27的仓库产物存在；不依赖任何前序Agent口头总结。

本步不趁验收新增范围。修复阻塞缺陷可做最小改动；涉及产品决策先更新规格/ADR再实施。

## 产物

- `docs/verification.md`：每个需求ID→自动/手工测试→证据/结果/环境
- `docs/release-checklist.md`
- 更新 `docs/development.md`、`README.md` 和与实现不符的现行专题/ADR状态
- 最终测试、性能、安装、隐私、无障碍报告和发布产物校验和

## 验收矩阵

必须逐项覆盖：

1. 单Workspace、Atlas/Efforts/Archive、LocalAppData/便携权威源。
2. 纯可视化GFM、最小写回、Raw原样、UTF-8/BOM/CRLF、中文IME。
3. Ctrl+S、关闭三分支、保存竞争、真正外部覆盖丢弃、自写事件、Task局部提交与脏正文。
4. 专有ignore、链接循环/alias、Archive默认排除、全局索引重建、中文1/2/长词搜索。
5. 基础Task且9最高；无status/日历/ICS/重复/依赖。
6. GPUI Kit精确版本/feature与无直接GPUI覆盖、Dock Pane/Tab持久化、Homepage、状态恢复、图片/PDF恶意输入与worker故障。
7. Pi JSONL、cwd/session、显式上下文、Extension UI、退出/中止、非干涉与无自动注入。
8. Zed主题→Kit Theme/应用扩展快照、对比度、keymap last-good、Kit Command实际键位、zh/en、标准与自定义控件键盘路径、Narrator、200%缩放。
9. 本地诊断隐私、性能预算、当前用户安装/升级/卸载、第三方许可。
10. 明确验证没有V1外功能被误宣称。

## 统一命令

在适用环境执行并记录原始结果：

```powershell
pwsh -File tools/doctor.ps1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked --target x86_64-pc-windows-msvc
python tools/check-docs.py
pwsh -File tools/bench.ps1
pwsh -File tools/smoke-install.ps1
```

CI不能代替目标Windows 11实机的IME/GPU/PDF/Narrator/安装测试。因硬件/权限跳过的项目标为未验证，不算通过。

## 发布阻塞条件

任何任务写错文件、Raw数据破坏、外部变化被覆盖、Pi配置被修改/隐式注入、诊断泄密、安装器删除Workspace、许可证缺失，均阻塞。性能/无障碍未测或未达标必须如实标记，不能改文案为已完成。

## 完成定义

需求矩阵无“实现声称但无证据”的核心项；已知限制准确；release artifacts、hash、NOTICE和报告齐全。结束报告列所有改动、命令结果、手工证据、跳过/失败和发布结论。