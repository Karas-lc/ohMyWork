---
title: "ADR-0004: Use iCalendar semantics for Markdown tasks"
status: "Superseded"
date: "2026-10-01"
authors: "Karas"
tags: ["architecture", "decision", "tasks", "icalendar", "markdown"]
supersedes: ""
superseded_by: "ADR-0009"
---

# ADR-0004: Use iCalendar semantics for Markdown tasks

## Status

**Superseded** by [ADR-0009](adr-0009-basic-tasks-calendar-v3.md) on 2026-10-02.

以下是历史方案，不再约束 V1。V1 只有 checkbox、start/due 日期及 priority 1–9（9 最高）；日历与 ICS 延期 V3，标准映射届时重新决策。

## Context

V1 requires local Tasks, Calendar views, recurrence, dependencies, and explicit ICS import/export. Tasks must remain editable Markdown. Dataview inline fields provide a readable query-friendly shape, but Dataview itself does not define task recurrence behavior. Obsidian Tasks adds natural-language recurrence and plugin-specific conventions, while iCalendar already defines interoperable `VTODO`, dates, status, recurrence, exceptions, and relationships.

## Decision

A Task is a Markdown checkbox with Dataview inline-field syntax. Field names and behavior use iCalendar properties as the canonical direction, including `UID`, `DTSTART`, `DUE`, `RRULE`, `STATUS`, `PRIORITY`, `COMPLETED`, `RELATED-TO`, `RDATE`, `EXDATE`, and `RECURRENCE-ID` as applicable.

Recurrence follows RFC 5545 rather than Obsidian Tasks natural-language rules. Relationships use RFC 5545 and RFC 9253. SQLite stores a rebuildable projection, not the authoritative Task.

The exact Markdown encoding of values, parameters, exceptions, and write-back operations requires follow-up decisions before implementation.

## Consequences

### Positive

- **POS-001**: Recurrence and time semantics have an external standard and conformance vocabulary.
- **POS-002**: Explicit ICS import/export requires less lossy translation.
- **POS-003**: Tasks remain visible and editable in ordinary Markdown and queryable as Dataview fields.
- **POS-004**: Unknown iCalendar properties can be preserved instead of collapsed into a proprietary task model.
- **POS-005**: RFC 9253 provides explicit relationship direction and project-style temporal relation types.

### Negative

- **NEG-001**: RRULE and iCalendar DATE-TIME rules are less approachable than natural-language recurrence.
- **NEG-002**: Time zones, floating time, exceptions, instance edits, and relationship direction are complex.
- **NEG-003**: Direct iCalendar property names differ from common Obsidian Tasks field names.
- **NEG-004**: Markdown has no standard representation for iCalendar property parameters, requiring a documented encoding.
- **NEG-005**: Full lossless import may require preserved side data for properties not representable inline.

## Alternatives Considered

### Obsidian Tasks semantics and Emoji format

- **ALT-001**: **Description**: Use Emoji dates and natural-language rules such as `every day when done`.
- **ALT-002**: **Rejection Reason**: The owner explicitly selected Dataview fields rather than Emoji and prioritized ICS/RRULE behavior.

### Friendly aliases mapped to iCalendar

- **ALT-003**: **Description**: Author `[id::]`, `[start::]`, `[repeat::]`, and `[completion::]`, then map them to iCalendar properties.
- **ALT-004**: **Rejection Reason**: Direct iCalendar names minimize hidden mapping and make round-trip semantics clearer.

### Database-native tasks

- **ALT-005**: **Description**: Store Tasks as SQLite records and render or export Markdown secondarily.
- **ALT-006**: **Rejection Reason**: It violates the open-file source-of-truth decision and weakens external editing.

### Proprietary recurrence format

- **ALT-007**: **Description**: Define an ohMyWork-specific recurrence DSL optimized for the UI.
- **ALT-008**: **Rejection Reason**: It creates avoidable conversion and long-term compatibility costs.

## Implementation Notes

- **IMP-001**: Freeze the Markdown value/parameter encoding in a follow-up ADR before storage or editor code.
- **IMP-002**: Build RFC fixture tests for DATE, DATE-TIME, TZID, RRULE, RDATE, EXDATE, RECURRENCE-ID, folding, and escaping.
- **IMP-003**: Preserve source ranges and file versions for safe Task write-back.
- **IMP-004**: Keep Markdown parsing, internal Task types, recurrence expansion, and ICS serialization as separate layers.
- **IMP-005**: Surface invalid or unsynchronized DTSTART/RRULE combinations instead of guessing.

## References

- **REF-001**: `docs/task-calendar.md`
- **REF-002**: RFC 5545, Internet Calendaring and Scheduling Core Object Specification
- **REF-003**: RFC 9253, Support for iCalendar Relationships
- **REF-004**: `docs/adr/adr-0002-markdown-source-and-hybrid-persistence.md`
