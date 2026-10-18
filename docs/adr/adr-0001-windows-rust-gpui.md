---
title: "ADR-0001: Build V1 as a Windows-native Rust/GPUI application"
status: "Accepted"
date: "2026-10-01"
updated: "2026-10-02"
authors: "Karas"
tags: ["architecture", "decision", "rust", "gpui", "windows"]
supersedes: ""
superseded_by: ""
---

# ADR-0001: Build V1 as a Windows-native Rust/GPUI application

## Status

**Accepted**

## Context

ohMyWork targets a high-intensity personal workstation where startup, input, navigation, search, and dense multi-pane interaction are product capabilities. The owner selected Rust as the primary language and Zed's GPUI as the UI framework. Shipping multiple desktop platforms in V1 would expand testing and packaging before the core interaction model is validated.

## Decision

V1 will target Windows 11 x64, implemented primarily in Rust with GPUI. Architecture may avoid unnecessary Windows coupling, but V1 makes no Windows 10, ARM64, macOS, or Linux delivery promise.

Distribution uses a per-user installer without administrator privileges, shortcuts and an uninstall entry. Upgrades are manual; V1 has no automatic updater.

Performance-sensitive parsing, indexing, state, and process integration remain in Rust. The UI uses native GPUI components rather than a browser-based application shell.

## Consequences

### Positive

- **POS-001**: Rust provides explicit ownership and concurrency tools for long-lived document, index, and subprocess state.
- **POS-002**: GPUI supports the dense, low-latency interaction model that motivates the product.
- **POS-003**: A single V1 platform keeps accessibility, packaging, input, file watching, and process behavior testable.
- **POS-004**: The architecture aligns with Zed-inspired interaction density without requiring a Zed clone.

### Negative

- **NEG-001**: GPUI is less mature and has a smaller ecosystem than mainstream desktop frameworks.
- **NEG-002**: Windows-specific IME, DPI, accessibility, packaging, and process behavior still require substantial work.
- **NEG-003**: Cross-platform support will require later validation and may expose hidden Windows assumptions.
- **NEG-004**: Some UI capabilities may need custom implementation instead of established web components.

## Alternatives Considered

### Electron or another browser shell

- **ALT-001**: **Description**: Build the workstation as a web UI packaged with Chromium and Node.js.
- **ALT-002**: **Rejection Reason**: It conflicts with the selected Rust/GPUI direction and adds a browser runtime to a product explicitly centered on native responsiveness.

### Tauri with a web frontend

- **ALT-003**: **Description**: Keep a Rust backend while rendering the UI in the system WebView.
- **ALT-004**: **Rejection Reason**: It preserves some Rust benefits but does not satisfy the chosen GPUI interaction and rendering foundation.

### Cross-platform V1

- **ALT-005**: **Description**: Treat Windows, macOS, and Linux as equal release targets from the first version.
- **ALT-006**: **Rejection Reason**: It multiplies packaging, accessibility, file-system, input, and Pi process integration work before the core product is proven.

## Implementation Notes

- **IMP-001**: Keep OS-specific discovery, path, shell, packaging, and accessibility integration behind adapters.
- **IMP-002**: Never block the GPUI UI thread on file, database, parsing, search, or subprocess I/O.
- **IMP-003**: Establish Windows DPI, IME, keyboard, and screen-reader test coverage before release.
- **IMP-004**: Validate the approved budgets in `docs/product-requirements.md` on the owner's measured reference machine. Targets are not measured results.

## References

- **REF-001**: `PRODUCT.md`
- **REF-002**: `docs/architecture.md`
- **REF-003**: `docs/product-requirements.md`
