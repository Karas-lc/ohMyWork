---
title: "ADR-0002: Keep Markdown authoritative and use hybrid persistence"
status: "Superseded"
date: "2026-10-01"
authors: "Karas"
tags: ["architecture", "decision", "markdown", "sqlite", "storage"]
supersedes: ""
superseded_by: "ADR-0007"
---

# ADR-0002: Keep Markdown authoritative and use hybrid persistence

## Status

**Superseded** by [ADR-0007](adr-0007-single-workspace-storage.md) on 2026-10-02.

以下保留历史理由，不作为现行规格。Markdown 权威与派生索引原则继续保留；多个 Workspace、默认不写身份文件等假设已被替代。

## Context

Users already have directories and Markdown files that must remain usable by editors, scripts, version control, and Pi. ohMyWork also needs fast search, links, Task queries, layouts, migrations, and recovery. Making either plain files or a database responsible for every concern would compromise one side of that requirement.

## Decision

Human-authored Pages and Tasks remain ordinary Markdown files in their existing Workspace directories and are the content facts.

Persistence is split by responsibility:

- open files for user-authored Markdown and future human-managed configuration;
- SQLite for rebuildable indexes and application-owned structured state;
- ordinary or content-addressed files for future large payloads and attachments;
- global Application Data for Workspace State by default;
- optional `.ohmywork/` Portable Mode only when the user explicitly enables it.

A Workspace is opened in place. ohMyWork does not import or duplicate the entire directory.

## Consequences

### Positive

- **POS-001**: Users retain access through any editor, script, backup, or version-control workflow.
- **POS-002**: Pi and ohMyWork operate on the same files without synchronization copies.
- **POS-003**: Search and navigation can use SQLite without turning indexes into irreplaceable user data.
- **POS-004**: Deleting or rebuilding derived data does not delete authored content.
- **POS-005**: Existing directories remain clean unless Portable Mode is explicitly selected.

### Negative

- **NEG-001**: External edits create concurrency and conflict cases that a database-only editor could avoid.
- **NEG-002**: File parsing, stable identity, renames, partial writes, and event storms require careful handling.
- **NEG-003**: Some structured operations require source-aware Markdown edits rather than simple database updates.
- **NEG-004**: Global and portable state introduce migration and reconciliation concerns.

## Alternatives Considered

### SQLite as the canonical content store

- **ALT-001**: **Description**: Store pages, blocks, and tasks as database records, with optional Markdown export.
- **ALT-002**: **Rejection Reason**: It would make external files secondary, weaken interoperability, and create migration pressure for existing workflows.

### Always copy directories into an application library

- **ALT-003**: **Description**: Import each opened directory into an ohMyWork-managed content root.
- **ALT-004**: **Rejection Reason**: It duplicates data and introduces ambiguous synchronization with the original source.

### Store every state file inside `.ohmywork/`

- **ALT-005**: **Description**: Make Workspace state portable by default.
- **ALT-006**: **Rejection Reason**: It modifies every opened directory and conflicts with non-invasive adoption.

## Implementation Notes

- **IMP-001**: Derived index rows must carry source identity and version so stale results cannot overwrite current state.
- **IMP-002**: File writes must detect external changes and use a safe replacement strategy.
- **IMP-003**: SQLite migrations, integrity checks, and rebuild behavior require automated tests.
- **IMP-004**: The exact global data layout, Workspace identity, and conflict algorithm require follow-up decisions.

## References

- **REF-001**: `docs/architecture.md`
- **REF-002**: `docs/product-requirements.md`
- **REF-003**: `CONTEXT.md`
