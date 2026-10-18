# 05 — 接入 GPUI Kit 与自定义段落编辑器

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` 第4节和 A11Y-001、`docs/architecture.md` 第2/8节，以及GPUI Kit的 [Element](https://gpui-kit.com/zh-CN/docs/element.md)、[测试](https://gpui-kit.com/zh-CN/docs/test.md) 和 [无障碍](https://gpui-kit.com/zh-CN/docs/accessibility.md)。确认 `omw-document` 的事务、视觉位置、Undo/Redo和最小patch测试通过；确认02的GPUI Kit应用与标准Input样例可运行。

只实现段落级编辑表面与输入/选择/无障碍路径；工具栏、按钮、菜单、Tooltip和滚动外壳优先复用GPUI Kit标准组件。无损可视化Markdown没有现成Kit控件，核心编辑面仍自定义；结构化GFM留到09，磁盘保存留到07。

## 产物

`crates/omw-app/src/editor/{mod,view,element,input,layout,selection,a11y}.rs`，以及必要测试。

## 状态与所有权

- `DocumentSession` 是共享 `Entity`；同一文件的多个 `EditorView` 引用同一实体。
- View 只持视口、视觉选择、marked range和布局缓存，不复制文档事实。
- 布局缓存键至少含 `NodeId`、内容版本、宽度、字体和主题版本。
- `ParsePool` 固定2线程；每个文档只保留最新待解析代次，结果回UI前核对doc revision。解析和大布局不得阻塞UI线程。

## 实现顺序

1. 已建立固定 2-worker `ParsePool`、结果代次过滤、按顶层块的可见投影，以及 GPUI `list` 的按视口布局/绘制；块内 shaping 命中布局只为已布局块生成。
2. 已通过 `gpui_kit::*` 重导出的 GPUI text system shaping 命中字形，再映射到 `NodeId +` 视觉位置；没有新增裸 GPUI 依赖。
3. 已将 `EntityInputHandler` 的 UTF-16 range 映射到可见字素位置；Markdown 标记、Raw/代码及包含 Raw 子节点的复杂块不进入光标空间。
4. 已实现插入、删除、方向/按词/按 grapheme 移动、跨行选择、复制粘贴、Undo/Redo；跨段选区先按领域事务合并可编辑段落，再执行替换/删除。
5. `replace_and_mark_text_in_range` 的组合中间态仍只留在 View；提交时通过共享会话写入。
6. TextInput 根节点现在提供稳定合成 TextRun 子节点、TextSelection、焦点、可读错误描述；普通段落和 Raw/复杂块均有可访问文本。
7. 文档变更只安排 GPUI 重绘；本切片不做磁盘 I/O。

## 验证

- 已自动覆盖：共享文档双视图同步、输入/IME 标记与提交、撤销/剪切、跨字素与 UTF-16 命中、跨段选区替换、Raw/复杂块原样可视化。
- `gpui_kit::list` 负责可变高块的视口虚拟化；长文布局不再把全文作为单个 shaped surface。
- 已运行 `cargo test -p omw-app --lib --locked`、`cargo test -p omw-app --test editor_surface --locked`、`cargo fmt --all -- --check` 和 `cargo clippy -p omw-app --all-targets --no-deps --locked -- -D warnings`。
- Windows 实机 IME/Narrator 仍需在带 Narrator 的 Windows 11 桌面执行；仓库内代码已暴露 TextInput、合成 TextRun、选区和错误描述，但不能把无头 Kit 测试当作实机验证。

## 完成定义

可在内存中打开fixture并流畅编辑段落，但不声称已支持全部GFM或保存。Kit的源码`Editor`/`TextView`不能代替源片段保真的领域模型；若输入映射必须泄露Markdown标记给光标，停止修映射，不用源码模式规避。