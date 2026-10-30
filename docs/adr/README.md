# ADR 索引

Accepted 表示已确认设计，不表示已有实现。Superseded 文档仅用于解释历史权衡，不能覆盖当前需求。

| ADR | 状态 | 主题 |
|---|---|---|
| [0001](adr-0001-windows-rust-gpui.md) | Accepted，已补充 | Windows 11 x64 / Rust / GPUI / 当前用户安装包 |
| [0002](adr-0002-markdown-source-and-hybrid-persistence.md) | Superseded by 0007 | 历史文件/多 Workspace 存储假设 |
| [0003](adr-0003-pi-official-rpc-boundary.md) | Superseded by 0010 | 历史 Pi RPC 与 Workspace 绑定 |
| [0004](adr-0004-icalendar-task-semantics.md) | Superseded by 0009 | 历史 V1 iCalendar 任务模型 |
| [0005](adr-0005-zed-theme-import-adapter.md) | Accepted，已补充 | 主题语义适配、手动导入与安全覆盖 |
| [0006](adr-0006-out-of-process-plugins.md) | Proposed，V2 | 进程外插件方向 |
| [0007](adr-0007-single-workspace-storage.md) | Accepted | 单一 Workspace、Atlas/Efforts/Archive、全局索引 |
| [0008](adr-0008-visual-editing-save-policy.md) | Partially superseded by 0011/0012 | 编辑模式已替代；DEC-004…007 保存边界仍现行 |
| [0009](adr-0009-basic-tasks-calendar-v3.md) | Accepted | 基础 Task，日历与 ICS 延期 V3 |
| [0010](adr-0010-user-controlled-pi-context.md) | Accepted | 用户控制 Pi 目录、会话与上下文，宿主不干涉 |
| [0011](adr-0011-markdown-source-editor.md) | Accepted，部分替代 0008；被 0012 部分替代编辑模式 | 整篇 Markdown 源码编辑与严格 Task 源范围保护 |
| [0012](adr-0012-inline-markdown-editing.md) | Accepted，部分替代 0011 | 原生 GPUI 同区 Markdown 渲染编辑（必要位置显露标记） |

仍有实现前待定项，见 [产品需求](../product-requirements.md)。
