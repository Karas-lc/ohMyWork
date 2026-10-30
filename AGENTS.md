# AGENTS.md

本文件约束参与 ohMyWork 的人类与 AI 代理。当前仍在设计阶段，文档中的目标不表示功能已经实现。

## 产品哲学

协作不是要求他人改变工作方式来适应 ohMyWork，而是让 ohMyWork 主动兼容现有工具、人员与流程，以尽可能低的摩擦、尽可能快地调动更多资源投入生产。

不建设团队协作平台，不强迫迁移私有格式，不伪造外部兼容性，不接管用户 Pi。

## 当前范围

- V1：Windows 11 x64、Rust/GPUI、本地工作台、基础任务、原生 GPUI 同区 Markdown 渲染编辑、图片/PDF 预览与 Pi 界面适配。
- V2：插件、外部连接器、资源信封、自动化、Dataview 功能级兼容。
- V3：日历、ICS；复杂任务规则须重新决策。

V1 不实现 Live Preview、独立全局源码模式、多 Workspace 系统、专门项目对象、日历、ICS、重复/依赖、反向链接或重命名链接更新。

## 已确认约束

1. **一个 Workspace**：Atlas、Efforts、Archive 为根目录中的真实文件夹。项目/领域只是普通目录，不再设计多个 Vault 的注册、切换或跨 Workspace 搜索。
2. **开放内容**：Markdown 是页面与任务事实来源，CommonMark + GFM；Dataview 字段仅是基础 Task 的专项扩展。
3. **可视化编辑**：原生 GPUI 同区 Markdown 渲染编辑（平时隐藏已识别标记，活动 leaf 原位显露必要源）；无源码切换；未知/不能安全可视化的内容保留为 Raw 节点，允许移动/删除，不编辑内部文本。只改写必要源范围。
4. **正文保存**：Ctrl+S/保存命令提交；关闭脏内容提供保存/放弃/取消；不持久化未保存草稿。
5. **外部优先**：真实外部修改直接重载并丢弃未保存本地编辑，无撤销恢复。应用自写事件不得误判成外部修改。
6. **任务保存**：任务组件立即只写任务范围，并合并到内存文档，保留其他未保存正文；不得把此操作当作外部覆盖或顺带保存全文。
7. **安全写盘**：同目录临时文件 + Windows 原子替换；保存前校验磁盘版本，避免覆盖已变化源。失败不留下半文件。
8. **本地存储**：全局状态、设置、主题、索引、日志默认在 `%LOCALAPPDATA%\ohMyWork\`；一个全局 SQLite 索引。Workspace 自动创建 `.ohmywork/id`；便携模式才额外保存可迁移状态。
9. **文件发现**：只读取 `.ohmyworkignore`，不读取 `.gitignore`。按系统语义跟随符号链接/Junction，并防循环；这不是 Pi 访问控制。
10. **Pi 非干涉**：只适配用户安装的官方 `pi --mode rpc`；不安装、更新、限制或修改其配置，不自行规定 Agent 排队、插话或工具策略。
11. **上下文由用户选**：ohMyWork 不自动发送目录、搜索结果或任务内容；文件传路径，选区传文本。Pi 自己读取文件和指令仍按用户配置执行。
12. **性能与可访问性**：UI 线程不阻塞；采用需求文档中的已批准性能预算，WCAG 2.2 AA、键盘优先、中文/英文和 Windows 缩放。
13. **诊断**：仅本地日志与主动导出，无遥测/自动上传；默认不记录正文、Pi 对话或凭据。

## 文档职责与变更流程

- `README.md`：概览及项目状态。
- `PRODUCT.md`：用户、目的、设计原则。
- `CONTEXT.md`：领域词汇，不记录实现规格。
- `docs/product-requirements.md`：当前范围、验收和开放问题。
- `docs/architecture.md` 与专题文档：架构边界和行为契约。
- `docs/adr/README.md`：现行/历史 ADR 索引；Superseded 记录不是现行要求。
- `.agents/plans/`：检查点，不替代最新专题规格。

实现前阅读对应规格；有冲突先修正文档，不选择方便的旧版本。用户尚未决定的事项明确标为待定，不擅自扩展 V1。难以逆转的决策用 ADR 记录；实现采取可验证的纵向切片。

## 边界代码与工作约束

使用明确类型表达内部状态，JSON/RPC/主题解析放在适配器边界。后台结果必须检查代次并可取消；错误给出可操作诊断。不得用 `unwrap()`/`expect()` 处理用户输入、文件或进程触发的失败。

### 构建、打包与测试

- 不要自动构建或自动打包；只有用户明确要求时才执行。
- 不要编写测试，也不要运行测试。
- 无用的测试、测试运行和打包会占用太多时间；不要为了验证而自行增加这些工作。

## Agent skills

### Issue tracker

Issues and PRDs live as markdown files under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Use the default five triage status strings. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: root `CONTEXT.md` and `docs/adr/`. See `docs/agents/domain.md`.
