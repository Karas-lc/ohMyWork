---
title: "ADR-0006: Use out-of-process capability plugins in V2"
status: "Proposed"
date: "2026-10-01"
authors: "Karas"
tags: ["architecture", "decision", "plugins", "v2", "json"]
supersedes: ""
superseded_by: ""
---

# ADR-0006: Use out-of-process capability plugins in V2

## Status

**Proposed**

This direction was selected during V1 planning, but must be revalidated with V1 usage before the V2 protocol is frozen.

## Context

V2 is expected to connect browsers, email, WeChat, Feishu, and future products. Official integrations are preferably written in Rust, but Rust has no stable native ABI suitable for independently versioned dynamic plugins. Connectors are predominantly I/O-bound, may use fragile vendor SDKs, and should not crash the GPUI process. The product also needs a stable interoperability boundary that can eventually support non-Rust products.

Plugins are intended to provide resources and actions, not arbitrary application UI.

## Decision

V2 plugins will run as supervised independent processes. They will declare identity, protocol version, configuration Schema, capabilities, resources, triggers, actions, and commands through a manifest and a versioned JSONL/JSON-RPC-style protocol.

Official plugins should be implemented in Rust, but the wire contract is language-independent. Plugins cannot load arbitrary GPUI code into the host. The host renders configuration, resources, actions, and execution feedback through standard components.

V1 will not implement or pre-freeze this plugin host.

## Consequences

### Positive

- **POS-001**: Connector crashes and runtime conflicts are isolated from the main application.
- **POS-002**: Plugins can be updated and supervised independently.
- **POS-003**: A language-independent protocol allows integration with products whose SDKs are not available in Rust.
- **POS-004**: Standard host-rendered UI preserves accessibility, themes, and interaction consistency.
- **POS-005**: I/O-heavy connectors are unlikely to be materially affected by process-call overhead.

### Negative

- **NEG-001**: Process startup, lifecycle, backpressure, authentication, and diagnostics increase complexity.
- **NEG-002**: Declarative configuration and host UI cannot express every bespoke product experience.
- **NEG-003**: Protocol evolution and capability negotiation require long-term compatibility discipline.
- **NEG-004**: Process isolation alone is not a security sandbox; OS permissions still require an explicit model.
- **NEG-005**: Packaging multiple executables and vendor dependencies complicates distribution.

## Alternatives Considered

### Rust dynamic libraries

- **ALT-001**: **Description**: Load connector DLLs/CDylibs directly into the GPUI process.
- **ALT-002**: **Rejection Reason**: Rust lacks a stable native ABI, version coupling is high, and plugin crashes can terminate the workstation.

### WASM plugins

- **ALT-003**: **Description**: Run plugins in a portable sandboxed WASM runtime.
- **ALT-004**: **Rejection Reason**: Windows APIs, browser integration, vendor SDKs, OAuth callbacks, and long-lived network clients are constrained; WASM may still be evaluated for narrower automation later.

### Arbitrary GPUI extension code

- **ALT-005**: **Description**: Allow plugins to compile and inject custom GPUI pages and components.
- **ALT-006**: **Rejection Reason**: It couples plugins to Rust/GPUI versions and weakens process stability, theme consistency, and accessibility.

### Implement the host in V1

- **ALT-007**: **Description**: Build the protocol and a sample plugin before any real connector work.
- **ALT-008**: **Rejection Reason**: The V1 scope was deliberately reduced to the workstation and Pi; premature protocol work would freeze assumptions without usage evidence.

## Implementation Notes

- **IMP-001**: Revisit this ADR after V1 telemetry-free usage studies and before V2 implementation.
- **IMP-002**: Separate payload schemas from transport and version both capabilities and messages.
- **IMP-003**: Define process trust, secrets, filesystem/network access, cancellation, retries, and audit behavior before accepting third-party plugins.
- **IMP-004**: Build one real vertical connector before generalizing the SDK.
- **IMP-005**: Preserve untouched source JSON alongside any normalized Resource envelope.

## References

- **REF-001**: `docs/roadmap.md`
- **REF-002**: `docs/architecture.md`
- **REF-003**: `docs/product-requirements.md`
