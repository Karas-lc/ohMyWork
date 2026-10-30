---
title: "ADR-0012: 原生 GPUI 同区 Markdown 渲染编辑"
status: "Accepted"
date: "2026-10-27"
authors: "Karas"
tags: ["architecture", "editor", "gpui-kit", "markdown"]
supersedes: "ADR-0011 (editing mode only: DEC-001)"
superseded_by: ""
---

# ADR-0012: 原生 GPUI 同区 Markdown 渲染编辑

## Status

**Accepted**。部分替代 ADR-0011 的编辑模式（DEC-001）；DEC-002 的源字节原则和 DEC-004～006 的保存/Task 原则继续有效。产品需求与 AGENTS 描述已同步更新。

## Context

用户要求 Markdown 在同一区域呈现并可编辑：粗体等行内格式可见，标题按层级实际放大，GFM 表格显示为可编辑单元格，而不是源码＋预览分栏。实现采用 GPUI Kit 的公开排版/几何接口，并保留 ohMyWork 的 Markdown 原文字节、文档级撤销、Ctrl+S 保存、外部修改优先和任务局部提交。选择 GPUI Kit 底座、有限借鉴 Velotype，不引入 Velotype 或 zorite-editor 的文档/持久化引擎。

当前使用整篇源码编辑器暴露全部标记；新行为为“必要位置显露标记”：平时隐藏已识别 Markdown 标记，活动段落/行内/单元格在原位置显露原文以便编辑。标题进入编辑仍保持对应字号，表格进入编辑仍保持网格。

该行为改变现行 ADR-0011 的编辑模式，不改变其 DEC-002 的源字节原则和 DEC-004～006 的保存/Task 原则。

## Decision

- **DEC-001**：V1 采用原生 GPUI 同区 Markdown 渲染+编辑表面（`MarkdownElement` + `EditorSurface` + `EntityInputHandler`）。一个可连续导航的编辑表面；平时隐藏已识别 Markdown 标记；活动 leaf 可在原位置显露原文标记以编辑。
- **DEC-002**：原文是唯一事实来源。页面加载、聚焦、重排、换主题、选区移动不能使文档 dirty。既有 Raw/HTML/未知语法用原文呈现并可编辑，不执行 HTML；不借本次支持添加 Wiki Link、Properties、数学或 YAML 元数据语义。
- **DEC-003**：标题识别使用现有 ATX/Setext 解析；`++` 不是标题，保持原文。表格对齐与结构来自 Comrak AST，不由视觉反推。
- **DEC-004**：使用 `gpui-kit = "=0.8.1"`（固定 `gpui-pre = 0.3.10`）；必须使用 `gpui_kit::base::text::editor_host` 的公开接口。`TextViewStyle::with_heading` 属于 Base API。保留 editor 的 `tree-sitter-markdown` feature 仅用于活动原文样式识别，不用于表格渲染。
- **DEC-005**：派生 `ParsedProjection` / `ProjectedLeaf` 等仅视图状态，保存在 `EditorSurface`；不放进持久化或 SourceSession 历史。解析在后台可取消。
- **DEC-006**：所有确定输入统一走 `replace_source_range` → 仅 `PageCommand::ReplaceSource`；Composition/preedit 只叠加到表面展示 Rope，不进入 PageDocument。IME 遵循 EntityInputHandler 契约。
- **DEC-007**：保存、Task 局部提交、外部修改优先、Undo/Redo 仍由现有 `DocumentStore` / `SourceSession` / `PageCommand::Undo/Redo` 收口。表面不拥有第二份历史。
- **DEC-008**：表格 cell 编辑范围基于 Comrak table_cell scanner（奇数反斜杠转义的 | 不分列）；短行缺失 cell 首次输入时物化 slot；单 cell 禁止多行粘贴。
- **DEC-009**：不提供全局源码模式、独立预览窗或模式切换按钮。链接点击默认放置 caret；Ctrl+点击仅沿已有安全路由。

## Consequences

### Positive

- **POS-001**：用户在同一表面看到格式化标题、粗体、表格网格，同时可直接编辑源；降低上下文切换。
- **POS-002**：源字节保真、文档级撤销、Ctrl+S、外部优先、Task 局部写盘原则不变。
- **POS-003**：复用 GPUI Kit 公开排版/几何/输入宿主，避免私有复制或全量第三方编辑引擎。

### Negative

- **NEG-001**：实现复杂度转移到投影映射（source <-> display offset）、几何收集、活动 leaf 显露、表格 slot 物化与 IME 组合。
- **NEG-002**：键盘导航、选区、IME 必须正确桥接显示几何与源字节；跨版本 leaf ID 复用需小心。
- **NEG-003**：升级 gpui-kit 引入了 gpui-fast 等新传递依赖（未启用）；需人工验证 Windows IME、中文、缩放、横向滚动一致性。

## Alternatives Considered

### 继续纯源码编辑器

- **ALT-001**：保持现有源码控件，标记始终可见。用户明确要求格式化呈现与编辑在同区，不接受。
- **ALT-002**：双栏源码+预览。违反“同一区域”要求，增加维护。

### 采用 Velotype / zorite 全编辑引擎

- **ALT-003**：引入外部 Markdown 文档模型与持久化。违反“选择 GPUI Kit 底座、有限借鉴 Velotype，不引入 ... 的文档/持久化引擎”和“原文是唯一事实来源”。

### 结构化整文档序列化 WYSIWYG

- **ALT-004**：把所有输入转成结构树再序列化回源。违反“源码保真优先”和“用户没有要求严格 WYSIWYG，采用 Obsidian 式必要位置显露标记”。

## Implementation Notes

- **IMP-001**：新增 `crates/editor/src/view/{projection.rs, surface.rs, input.rs, tables.rs}`。`parse_for_view` 在一次 Comrak 遍历中收集 ViewMetadata。
- **IMP-002**：`LeafId` 表面内分配；`SourceRun` 携带 `MappingKind::{Verbatim,Decoded,Hidden}`。Bias 处理隐藏标记边界。
- **IMP-003**：`replace_source_range` 在 clone 的 SourceRope 上局部 patch，然后唯一一次 `PageCommand::ReplaceSource`。
- **IMP-004**：移植 Velotype `text_offsets.rs` 的 6 个纯 UTF 转换函数到 input.rs，标注来源 commit `07a24a9b5b953539dede8e61a1f7d9d0adbba590`，Apache-2.0。
- **IMP-005**：`MarkdownElement` 用 `Paragraph` + `BlockNode` + `Table` + `with_leaf_key` + `collect_leaf_geometry`。
- **IMP-006**：标题字号：H1 2.0 ... H6 1.0，semibold；通过 `with_heading` 应用到排版。
- **IMP-007**：验证前置：用户明确授权后才 `cargo build --locked -p ohmywork --bin ohmywork`；否则交付标注“未进行原生界面验证”。
- **IMP-008**：不编写/运行测试；真实保存验收用工作台临时 Markdown 文件（含 BOM+CRLF、无外管道表、Setext）；demo 只用于样例展示。
- **IMP-009**：边界：空文件、未闭合、escaped pipe、未知 HTML、`++` 文本、跨块删除后语法不闭合同样可编辑保留原文。

## References

- **REF-001**：ADR-0011（部分替代编辑模式）
- **REF-002**：[GPUI Kit 0.8.1 editor_host 公开 API](https://docs.rs/gpui-base/0.8.1/gpui_base/text/editor_host/index.html) 与 [Paragraph/BlockNode](https://gpui-kit.com/base/text-view)
- **REF-003**：[Velotype text_offsets (仅函数，Apache-2.0)](https://github.com/manyougz/velotype/blob/07a24a9b5b953539dede8e61a1f7d9d0adbba590/src/text_offsets.rs)
- **REF-004**：本计划内联及 `local://native-markdown-editor-plan.md`
- **REF-005**：更新后的 `docs/product-requirements.md` EDIT-* 节
