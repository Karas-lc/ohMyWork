---
title: "ADR-0010: Pi 保持用户控制，宿主仅适配界面与协议"
status: "Accepted"
date: "2026-10-02"
authors: "Karas"
tags: ["architecture", "pi", "interoperability"]
supersedes: "ADR-0003"
superseded_by: ""
---

# ADR-0010: Pi 保持用户控制，宿主仅适配界面与协议

## Status

**Accepted**。替代 [ADR-0003](adr-0003-pi-official-rpc-boundary.md)；官方 RPC 与用户独立信任边界不变，撤销 Workspace/cwd/唯一会话绑定。

## Context

应用只有一个内容根目录，但 Pi 可用于用户选择的任意目录。此前把内容组织、上下文注入和 Agent 策略混在一起，违背“不干涉用户 Pi”的产品原则。

## Decision

- **DEC-001**：Rust 启动用户安装的 `pi --mode rpc`；通过 PATH/显式路径或命令发现，不捆绑、更新或配置 Pi，不自动扫描/修改 WSL。
- **DEC-002**：用户指定 cwd、选择新建或恢复，不按 Workspace/Effort 固定或自动恢复唯一 Session。
- **DEC-003**：原生可调侧栏展示会话和 Extension UI。显式关闭时官方 abort 后有序退出；无暂停承诺。
- **DEC-004**：用户显式选择附加内容，发送前可查看/移除。文件传路径、选区传文本，不自动注入已索引知识、Task 或 Archive 内容。
- **DEC-005**：Pi 自己读文件/指令、执行工具与队列行为依用户配置；宿主不规定默认排队/插话策略，不做工具权限拦截。

## Consequences

### Positive

- **POS-001**：用户既有 Pi 工作流保持独立，内容目录模型不强迫 Agent 采用同一边界。
- **POS-002**：显式附加避免宿主无意把全库内容塞入提示。
- **POS-003**：使用官方协议，避免 Node sidecar 和自建 Agent 运行时。

### Negative

- **NEG-001**：需要用户安装兼容 Pi，宿主不能通过自动改配置修复差异。
- **NEG-002**：无法承诺阻止 Pi 自行读取或上传内容；工具事件不是事前审批。
- **NEG-003**：目录和会话选择需要清楚呈现，不能用固定绑定简化所有恢复情况。
- **NEG-004**：RPC 不等价于完整 Pi TUI，部分能力需要诚实降级。

## Alternatives Considered

- **ALT-001**：固定 Workspace cwd 和唯一 Session。拒绝原因：用户选择自行指定目录、新建或恢复。
- **ALT-002**：宿主控制 queue/steer、工具权限和加载策略。拒绝原因：这些是用户 Pi 的行为，不由工作站另定政策。
- **ALT-003**：自动附加 Atlas/Efforts 或搜索结果。拒绝原因：用户决定哪些内容进入 Pi。
- **ALT-004**：SDK sidecar/重做 Rust Agent。拒绝原因：增加私有协议和运行时，不符合官方 RPC 边界。

## Implementation Notes

- **IMP-001**：遵循 JSONL、异步响应、message_end/agent_settled 等协议语义；正确处理被扩展直接消费而未启动的 prompt。
- **IMP-002**：不硬编码未经核验的能力握手；不试执行模型/工具来探测。
- **IMP-003**：视觉收起与显式关闭、历史候选、恢复 cwd 不匹配等交互仍待决定。
- **IMP-004**：日志默认不含对话/凭据；真正 Pi 运行可能依用户配置产生副作用，不能承诺零读写/零网络。

## References

- **REF-001**：[Pi 集成](../pi-integration.md)
- **REF-002**：[产品需求](../product-requirements.md)
