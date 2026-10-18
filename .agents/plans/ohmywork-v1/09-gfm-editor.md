# 09 — 完成 GFM 可视化编辑

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` EDIT/SAVE、`docs/adr/adr-0008-visual-editing-save-policy.md` 和GPUI Kit组件目录。确认段落编辑、最小序列化、安全保存和外部重载都已通过；本步不得绕过它们直接改字符串或写盘。

目标是第一个可用检查点：选目录→打开 Markdown→纯可视化编辑→Ctrl+S→外部修改刷新。

## 产物

- 使用GPUI Kit Button/Toolbar/Menu/Popover/Dialog构建的 `editor/toolbar.rs`，以及各 `NodeKind` 的渲染/输入适配
- `omw-document` 的结构事务和最小序列化扩展
- CommonMark/GFM、Raw、结构选择回归夹具

## 实现范围与顺序

1. heading、paragraph、quote；设置块类型。
2. 有序/无序列表，插入、缩进、反缩进、拆分/合并 item。
3. Task checkbox（只作为当前文档编辑；跨组件即时提交留到14）。
4. fenced/inline code；内部按可视文本编辑，但正确保留/选择安全 fence。
5. GFM table：插入表、编辑 cell、增删行列；结构变化只扩大到必要 table 容器。
6. link/image：可视标签、目标编辑与点击；不得执行原始 HTML。
7. strikethrough/autolink；已有 Bold/Italic。
8. Raw 节点只显示安全占位/摘要，允许整体选中、移动、删除，不允许内部编辑。
9. 普通粘贴始终是文字；单独“粘贴 Markdown”命令才解析并插入结构。HTML剪贴板不得直接执行。
10. 跨块选择、删除、移动和 Undo/Redo 保持 NodeId/源范围契约。

## 非目标

无源码模式、Live Preview、Wiki link、Callout、Properties/frontmatter语义、反向链接或全文 formatter。不能以外部编辑按钮冒充内置支持。

## 验证

- CommonMark/GFM fixture 的显示、编辑、保存、重开语义一致。
- 单一结构修改不改变无关段落；未知/Raw bytes 原样。
- 表格、列表缩进、代码 fence、多字节跨块选择、Markdown粘贴和Undo。
- 手工走完整检查点；外部编辑器修改打开文件后工作区立即重载。

## 完成定义

所有声明支持的节点均有可视操作和最小写回；标准交互外壳复用Kit组件，自定义部分只服务无损编辑；不支持内容稳定成为Raw。发现源保真/IME/Raw数据损坏时停止扩展并修根因，不能用Kit TextView/源码Editor规避或进入10。