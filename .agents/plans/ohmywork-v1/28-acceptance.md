# Plan 28 acceptance checkpoint

2026-10-18 修正：原“所有 1–27 计划和测试已通过”的结论缺少端到端证据，撤回。计划文件不替代现行规格或实际验收记录。

- 已修复并在原生工作台验证：真实文件编辑、正文保存、关闭三分支、外部覆盖、任务局部提交、Pi RPC fixture、PNG/PDF、Homepage 交互与真实路径恢复。
- 全量回归和生产二进制构建的实际结果见 `docs/verification.md`。
- 性能 runner 只报告后端索引 / 搜索 API；旧 GUI 代理指标作废，不宣称产品预算通过。
- 官方 Pi / 模型兼容、IME / Narrator / 缩放、安装升级卸载、严格 lint、CI 和 GUI 性能预算仍须独立验收。
- 不宣称 V1 发布验收完成，不沿用“计划已执行”作为通过证据。

现行证据：`docs/verification.md`。发布门禁：`docs/release-checklist.md`。
