# 13 — 实现基础 Task 解析与列表

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` 第5节、`docs/task-calendar.md`、`docs/adr/adr-0009-basic-tasks-calendar-v3.md`，以及GPUI Kit Checkbox/DatePicker/VirtualList文档。确认文档解析有checkbox/source range，索引可存Task投影，文件规则可过滤Archive/ignore。

本步做解析、投影、列表和编辑意图；真正局部落盘在14。

## 唯一模型

- Task 是 `- [ ]`/`- [x]` 与标题；checkbox 是唯一完成状态，没有 status。
- 可选 `[start:: YYYY-MM-DD]`、`[due:: YYYY-MM-DD]`、`[priority:: 1..9]`；9最高。
- 无时间、时区、提醒、重复、依赖、UID、日历或ICS。
- Markdown是事实来源，SQLite仅投影。

## 产物

- `omw-document/src/task.rs` 的完整字段/span/诊断解析
- `omw-app/src/ui/tasks.rs`
- Task fixtures 与索引投影测试

## 实现顺序

1. 仅在 Task语义文本范围识别字段，跳过 inline/fenced code 中伪字段；保留每个字段精确 source span。
2. 严格解析日期和1..9优先级。未知字段原样保留，无行为。
3. 无效或重复字段保留原文并生成诊断；该字段不参与类型排序，禁止偷偷取最后一个。
4. 列表支持完成、start/due、priority、路径/分区过滤和跳转源任务；Archive默认排除，可显式包含。
5. 默认排序：未完成优先→priority降序→due升序→路径/source位置稳定；缺失/无效值排序规则明确且测试固定。
6. UI优先使用Kit `Checkbox`、`DatePicker`和受限优先级选择控件；它们只产生 `TaskIntent`，不持有第二份Task事实。本步在没有14服务时不得假装保存成功。
7. 为单字段修复命令定义类型化意图，供14写回。

## 测试

普通/嵌套/重复标题Task；完成状态；1与9方向；无status；缺失、无效、重复、未知字段；代码伪字段；Archive/ignore；稳定排序与定位。用Kit `test-support`验证Checkbox/DatePicker的键盘、状态和无障碍值，但模型断言仍以TaskIntent/Markdown投影为准。

## 完成定义

列表完全可从 Markdown/索引重建，没有第二份Task状态。UI只能展示磁盘事实或清晰的待提交状态，不能先勾选后静默失败。