---
title: "ADR-0009: V1 基础任务，日历与 ICS 延期 V3"
status: "Accepted"
date: "2026-10-02"
authors: "Karas"
tags: ["architecture", "scope", "tasks"]
supersedes: "ADR-0004"
superseded_by: ""
---

# ADR-0009: V1 基础任务，日历与 ICS 延期 V3

## Status

**Accepted**。替代 [ADR-0004](adr-0004-icalendar-task-semantics.md)，不是继续要求 V1 兼容 iCalendar。

## Context

讨论 ICS/RRULE、时区与任务关系使首版偏离基础工作台。用户明确日历价值不足以进入 V1，后续再复杂化；最终任务字段也与原 iCalendar 原生属性及优先级方向不同。

## Decision

- **DEC-001**：V1 Task 仅 checkbox、标题、可选 start/due 日期与 priority。
- **DEC-002**：start/due 为 `YYYY-MM-DD`；priority 为 1–9，9 最高。无独立 status、时间、时区、提醒、重复或依赖。
- **DEC-003**：字段保留 Dataview 内联外形，普通 Markdown 是事实来源，SQLite 可重建。
- **DEC-004**：日历与 ICS 导入导出延期 V3；RRULE、关系、标准映射届时重新设计，不构成 V1 发布门槛。
- **DEC-005**：V2 Dataview 功能级兼容目标独立保留；DQL CALENDAR 查询与 V3 日历产品的边界届时明确，不暗中恢复 V1 日历。

## Consequences

### Positive

- **POS-001**：移除时区、实例异常、ICS 往返等首版成本，聚焦基础可用工作台。
- **POS-002**：用户可直接用 Markdown 阅读和维护日期/优先级。

### Negative

- **NEG-001**：V1 不能直接交换日历或执行重复/依赖任务。
- **NEG-002**：priority 9 最高与 iCalendar 方向相反，未来必须明确映射，不能按原值导出。
- **NEG-003**：若未来需要稳定任务身份和实例历史，需独立迁移设计。

## Alternatives Considered

- **ALT-001**：维持 VTODO/RRULE/RELATED-TO 为 V1 规范模型。拒绝原因：超出基础需求。
- **ALT-002**：仅 checkbox，不做日期/优先级。拒绝原因：用户保留 start/due 和 1–9 优先级。
- **ALT-003**：独立 status 枚举。拒绝原因：用户明确没有独立状态，checkbox 足够。

## Implementation Notes

- **IMP-001**：未知字段原样保留，不因为不执行就删除。
- **IMP-002**：写回遵守 ADR-0008 的 Task 局部立即提交契约。
- **IMP-003**：不要预建日历组件、ICS 库、RRULE 引擎或依赖图。

## References

- **REF-001**：[Task 规格](../task-calendar.md)
- **REF-002**：[路线图](../roadmap.md)
- **REF-003**：[保存边界](adr-0008-visual-editing-save-policy.md)
