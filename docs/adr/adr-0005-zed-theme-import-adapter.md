---
title: "ADR-0005: Import Zed themes through a semantic adapter"
status: "Accepted"
date: "2026-10-01"
updated: "2026-10-02"
authors: "Karas"
tags: ["architecture", "decision", "themes", "zed", "accessibility"]
supersedes: ""
superseded_by: ""
---

# ADR-0005: Import Zed themes through a semantic adapter

## Status

**Accepted**

## Context

The owner wants to use the same Rainbow theme currently used in Zed and wants other users to retain their own visual preferences. Zed Theme Family JSON already includes broad UI, editor, syntax, terminal, accent, border, and text-state colors. However, its schema is versioned and evolves independently from ohMyWork. ohMyWork also has Task, Homepage, and Pi states that may not have direct Zed equivalents.

## Decision

ohMyWork will define a stable internal semantic Theme Token model. A version-aware import adapter will read Zed Theme Family JSON and map known fields into an immutable internal theme snapshot.

Unknown Zed fields are ignored with diagnostics. Missing internal Tokens use deterministic fallbacks. Source theme files remain read-only. The imported theme may influence the complete application, not only Markdown syntax, but cannot redefine component behavior or inject code.

V1 imports only on explicit user selection and copies the original JSON into LocalAppData. It does not scan Zed directories or watch the source. Reimport is required to update. Critical low-contrast tokens receive automatic safety overrides with a visible change list; the source JSON remains unchanged.

## Consequences

### Positive

- **POS-001**: Existing themes such as Rainbow can be reused without manual color transcription.
- **POS-002**: UI components depend on stable semantic roles rather than an external schema.
- **POS-003**: Zed schema evolution is isolated in adapters and compatibility tests.
- **POS-004**: ohMyWork-specific Task, Homepage, and Pi states can receive intentional fallbacks.
- **POS-005**: Invalid themes can fail atomically without leaving a partially themed interface.

### Negative

- **NEG-001**: Mapping cannot guarantee pixel-identical results because component semantics differ.
- **NEG-002**: Multiple Zed schema versions require fixtures and ongoing adapter maintenance.
- **NEG-003**: Third-party themes may violate WCAG contrast for ohMyWork component combinations.
- **NEG-004**: Deterministic derived colors and fallback precedence must be documented and tested.
- **NEG-005**: Safety overrides intentionally change some imported colors; users must be shown which tokens changed.

## Alternatives Considered

### Make the Zed schema the internal model

- **ALT-001**: **Description**: Let every component read Zed style keys directly.
- **ALT-002**: **Rejection Reason**: It binds internal architecture to an independently evolving external schema and leaves ohMyWork-specific states undefined.

### Use only a proprietary theme format

- **ALT-003**: **Description**: Define an ohMyWork schema and require users to convert themes.
- **ALT-004**: **Rejection Reason**: It prevents direct use of the owner's Rainbow theme and adds unnecessary migration work.

### Copy colors into settings manually

- **ALT-005**: **Description**: Offer a color picker without importing Theme Family files.
- **ALT-006**: **Rejection Reason**: It loses theme identity, variants, syntax coverage, and portability.

## Implementation Notes

- **IMP-001**: Use read-only parsing with file-size, nesting, and collection limits.
- **IMP-002**: Build snapshots off the UI thread and apply them atomically.
- **IMP-003**: Test real Rainbow light/dark files, missing fields, unknown fields, and future schema versions.
- **IMP-004**: Implement and test the selected automatic safety-override policy; exact algorithms and applicable WCAG criteria remain to be specified.
- **IMP-005**: Always provide a keyboard-reachable built-in safe-theme recovery action.

## References

- **REF-001**: `docs/theme-system.md`
- **REF-002**: Zed theme schema, including `https://zed.dev/schema/themes/v0.2.0.json`
- **REF-003**: Rainbow Zed theme: `https://github.com/athxx/zed-theme-rainbow`
