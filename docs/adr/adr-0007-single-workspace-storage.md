---
title: "ADR-0007: 单一 Workspace 与三分区存储"
status: "Accepted"
date: "2026-10-02"
authors: "Karas"
tags: ["architecture", "workspace", "storage"]
supersedes: "ADR-0002"
superseded_by: ""
---

# ADR-0007: 单一 Workspace 与三分区存储

## Status

**Accepted**。替代 [ADR-0002](adr-0002-markdown-source-and-hybrid-persistence.md)，保留 Markdown 权威和混合存储原则，修正多 Workspace 假设。

## Context

此前把各项目理解成独立 Vault，导致多根注册、身份冲突与跨库查询设计过度。用户明确应用只有一个 Workspace，其中包含多个项目和领域；共享知识、当前投入与休眠内容应直接体现于目录。

## Decision

- **DEC-001**：唯一根目录下使用真实 Atlas/、Efforts/、Archive/。Efforts 内是普通文件夹，不引入项目对象或强制模板。
- **DEC-002**：Markdown 为页面/任务事实来源，一个全局 SQLite 索引保存可重建投影，不复制整个目录。
- **DEC-003**：自动创建 `.ohmywork/id`，而不是承诺默认零写入。布局/设置/主题/索引/日志默认在 `%LOCALAPPDATA%\ohMyWork\`；便携模式才额外写可迁移状态。
- **DEC-004**：读取专有 `.ohmyworkignore`，不读取 `.gitignore`。Archive 默认排除于搜索与任务，可显式纳入。
- **DEC-005**：按系统语义跟随链接目录并检测循环。这只影响宿主，不改变 Pi 权限或配置。

## Consequences

### Positive

- **POS-001**：用户、文件管理器与 Pi 均能直接理解目录结构，不依赖项目数据库。
- **POS-002**：移除多库管理，集中实现文件、索引与工作台。
- **POS-003**：共享知识与休眠内容的位置明确，活跃任务不被 Archive 干扰。

### Negative

- **NEG-001**：默认创建身份文件是明确的目录写入，不能继续宣传完全不修改根目录。
- **NEG-002**：全局数据库的损坏/迁移影响整个索引；需与不可重建设置隔离。
- **NEG-003**：跟随根外链接需要循环与别名去重处理，但不能借此限制 Pi。

## Alternatives Considered

- **ALT-001**：多个独立 Workspace/Vault。拒绝原因：不是用户的实际模型，增加无关复杂度。
- **ALT-002**：只用路径或全局 UUID 标识。拒绝原因：用户选择目录中的稳定 `.ohmywork/id`。
- **ALT-003**：每 Workspace 一个 SQLite。拒绝原因：单根模型采用一个全局索引。
- **ALT-004**：沿用 `.gitignore`。拒绝原因：工作台发现规则应独立于版本控制。

## Implementation Notes

- **IMP-001**：V1 的首次目录选择、缺失分区初始化、`.ohmywork/id` 与 LocalAppData 设置由 `project` 的文件/设置适配器和 Workspace 启动界面实现；便携数据集合仍留待后续里程碑，不构建多库注册器。
- **IMP-002**：索引重建不能删除持久布局、设置或导入主题。
- **IMP-003**：文件名/Markdown 正文搜索即可，不建立反向链接图。

## References

- **REF-001**：[需求](../product-requirements.md)
- **REF-002**：[架构](../architecture.md)
