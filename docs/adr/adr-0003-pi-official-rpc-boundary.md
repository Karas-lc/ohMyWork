---
title: "ADR-0003: Integrate user-installed Pi through the official RPC boundary"
status: "Superseded"
date: "2026-10-01"
authors: "Karas"
tags: ["architecture", "decision", "pi", "rpc", "security"]
supersedes: ""
superseded_by: "ADR-0010"
---

# ADR-0003: Integrate user-installed Pi through the official RPC boundary

## Status

**Superseded** by [ADR-0010](adr-0010-user-controlled-pi-context.md) on 2026-10-02.

以下是历史记录。官方 RPC 与 Pi 非干涉原则保留，但 Workspace 绑定 cwd/会话已撤销，改由用户选择目录、新建或恢复。

## Context

ohMyWork is implemented in Rust, while Pi's in-process SDK targets TypeScript/Node/Bun. Pi provides an official language-independent RPC mode using LF-delimited JSONL over stdin/stdout. The product must maximize compatibility with each user's existing Pi models, authentication, Extensions, Skills, Prompts, and tools without becoming responsible for installing or governing Pi.

Pi tools and Extensions run with the OS user's permissions. RPC tool lifecycle events are observational and are not a reliable pre-execution authorization boundary.

## Decision

A Rust `Agent Gateway` will supervise the user-installed `pi --mode rpc` process and adapt the official protocol into typed application state.

Each Pi Session binds to one Workspace and starts with that Workspace root as a stable `cwd`. ohMyWork will detect compatibility, present events, support official abort/close behavior, and report actionable errors. It will not bundle, install, update, repair, restrict, or rewrite Pi configuration.

Pi is treated as an independent trusted boundary. Future ohMyWork automation permissions do not govern Pi's internal tools.

## Consequences

### Positive

- **POS-001**: Uses Pi's supported language-independent integration instead of a private protocol.
- **POS-002**: Preserves user models, credentials, Extensions, Skills, Prompts, tools, and project discovery.
- **POS-003**: Process isolation prevents a Pi crash from directly crashing the Rust UI process.
- **POS-004**: A stable Workspace `cwd` keeps project instructions and session grouping understandable.
- **POS-005**: ohMyWork can evolve its UI without taking ownership of the Agent runtime.

### Negative

- **NEG-001**: Users must install and maintain a compatible Pi themselves.
- **NEG-002**: JSONL streaming, backpressure, event ordering, stderr, shutdown, and crash recovery add process complexity.
- **NEG-003**: RPC cannot reproduce every Pi TUI behavior exactly.
- **NEG-004**: ohMyWork cannot promise pre-execution approval for user-configured Pi tools.
- **NEG-005**: Native Windows and WSL discovery require an explicit compatibility policy.

## Alternatives Considered

### Node sidecar using the Pi TypeScript SDK

- **ALT-001**: **Description**: Ship or require a Node process that embeds the Pi SDK and exposes a custom protocol to Rust.
- **ALT-002**: **Rejection Reason**: It adds another runtime and private compatibility layer when official RPC already provides the required boundary.

### Reimplement the Agent runtime in Rust

- **ALT-003**: **Description**: Implement Pi behavior, models, tools, and sessions directly in ohMyWork.
- **ALT-004**: **Rejection Reason**: It creates an incompatible fork and makes ohMyWork responsible for rapidly changing Agent behavior.

### Restrict or intercept Pi tools

- **ALT-005**: **Description**: Rewrite tool configuration or inject an approval layer before each action.
- **ALT-006**: **Rejection Reason**: Official RPC events are not a reliable pre-execution barrier, and modifying tools contradicts the non-interference requirement.

### Dynamic multi-Workspace Session

- **ALT-007**: **Description**: Change a running Session between multiple Workspace roots.
- **ALT-008**: **Rejection Reason**: Pi resource discovery and project instructions depend on a stable working directory; switching makes behavior ambiguous.

## Implementation Notes

- **IMP-001**: Continuously consume stdout and stderr; never wait for a response while allowing either pipe to block.
- **IMP-002**: Treat `agent_settled`, not prompt acceptance, as authoritative idle/completion.
- **IMP-003**: Use a fake RPC subprocess for deterministic event, error, backpressure, and shutdown tests.
- **IMP-004**: Preserve unknown events for diagnostics and degrade safely across compatible Pi versions.
- **IMP-005**: Do not mutate Pi Session files; use official RPC/session operations and durable cursors.

## References

- **REF-001**: `docs/pi-integration.md`
- **REF-002**: `docs/architecture.md`
- **REF-003**: Pi RPC documentation distributed with `@earendil-works/pi-coding-agent`
