# 18 — 实现 GPUI Kit Dock 与标签

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` UI-001/002、`docs/architecture.md` 第4节，以及 GPUI Kit [Dock文档](https://gpui-kit.com/zh-CN/component/dock.md) 和 [测试文档](https://gpui-kit.com/zh-CN/docs/test.md)。确认文档由共享 `Entity<DocumentSession>` 管理，关闭脏文档已有保存/放弃/取消流程。

本步使用Kit Dock实现运行时Pane/Tab布局和内存dump/load；23才写入/恢复ui-state，19–20实现Homepage画布。禁止另建一套Pane树拖放/分割引擎。

## 数据边界

- UI行为使用 `gpui_kit::component::dock::{DockArea, DockLayout, DockAreaState, BasePanel, Panel}`。
- 每种标签由应用Panel适配：Markdown、Preview、Home。持久payload只含 `TabRef` 和视图状态；不含Entity指针、Markdown正文、Undo历史或位图。
- 应用包装 `PersistedDockState { schema_version, kit_layout_version, dock: DockAreaState }`，而不是把未经校验的Kit JSON直接视为整个应用状态。
- Markdown Panel按规范化 `DocumentKey`取得共享Session；同文档多个Panel只有视口/选择独立。

## 产物

- `omw-app/src/ui/{panes,tabs,shell}.rs`
- Markdown/Preview/Home Panel及固定panel name、恢复factory
- Dock dump/load适配与GPUI Kit UI集成测试

## 实现顺序

1. 用 `DockSkin::dock_area("main-dock", Some(layout_version), ...)` 创建应用中心Dock；Pi侧栏不伪装成文档Panel。
2. 实现各Panel的 `BasePanel`、`Panel`、Focusable/EventEmitter适配，用 `panel_handle`包装；所有可恢复Panel在初始化时注册factory。
3. 用 `DockLayout::tabs()`、`h_split()`、`v_split()`构造单组和任意嵌套布局；遵守最小尺寸。
4. 接入Kit现成Tab选择、重排、跨组拖放、分割和键盘操作；应用只监听 `DockEvent::LayoutChanged` 与业务事件。
5. 拦截/适配Panel关闭：只有最后一个引用脏文档的Panel关闭时进入保存/放弃/取消；取消必须让Dock保持原布局。应用关闭可批量处理。
6. 同文档多个Panel共享内容和保存状态；Ctrl+S只进入一次文档写队列。
7. `DockArea::dump`后先通过应用wrapper校验，再在内存load；缺失文件和未知/已移除Panel变为可关闭placeholder或安全默认。
8. 不在本步把状态写到磁盘，不恢复脏正文或自动启动Pi。

## 测试

- 用Kit `test-support`驱动真实Tab选择/重排、跨组拖放、横/纵分割、最大化/恢复和键盘操作。
- 多轮dump→load→dump后panel identity、选中项和布局语义稳定；坏version/未知panel安全失败。
- 最后Tab/组、最小窗口、拖放取消、关闭dirty的三分支。
- 同文档两Panel编辑同步、视口独立、只保存一次；中文标题和placeholder不破坏Dock。

## 完成定义

GPUI Kit Dock是Pane/Tab交互的唯一实现，应用只维护领域Tab引用、恢复factory和关闭政策。内存布局可验证往返；磁盘持久化仍留给23。