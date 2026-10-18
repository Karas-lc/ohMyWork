# 22 — 实现主题导入与安全覆盖

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/theme-system.md`、`docs/adr/adr-0005-zed-theme-import-adapter.md`、目标Zed Theme schema和GPUI Kit [Theme文档](https://gpui-kit.com/zh-CN/component/theme.md)。确认主要UI使用Kit Theme或应用语义扩展token；禁止组件直接读取Zed键。

本步只做手动主题导入/适配/对比度；不扫描Zed目录、不热监听来源、不分发无许可第三方主题。

## 产物

- `omw-app/src/theme/{mod,tokens,kit_adapter,zed_import,contrast,storage}.rs`
- 主题夹具与差异清单UI
- `%LOCALAPPDATA%/ohMyWork/themes/<import_uuid>/{source.json,metadata.json}`

## 固定契约

1. 内置light/dark语义快照分两层：映射到GPUI Kit `Theme`/tokens的标准window/panel/text/border/hover/active/selected/focus/error/warning/disabled，以及Markdown/Raw/Task/Canvas/Pi专有应用扩展token。两层由同一版本快照原子切换。
2. 显式映射至少含 `background→window/Kit background`、`text→text.primary/Kit foreground`、`panel.background→panel`、`editor.background→editor`；每个Kit和应用token都有确定的light/dark fallback。
3. 手动导入复制原JSON；metadata存family/member/schema/hash。重新导入同一条目保持import UUID。来源删除不影响副本。
4. 文件大小、深度、成员数有限；未知字段忽略，未知schema安全诊断；不联网取schema、不执行脚本。

## 对比度算法

按真实alpha与背景合成后的sRGB相对亮度检查。正文/选中文本至少4.5:1；焦点和必要非文本状态至少3:1。disabled采用明确产品可辨识目标，不谎称其适用正文AA。

背景不动；foreground分别向黑/白混合，用二分搜索求达标最小变化，选择改动更小者；必要时为focus加独立边线。记录 `original/resolved/reason`，只改内部快照，不改source.json。

## 流程与测试

后台解析到候选→映射/fallback→检查修正→生成Kit Theme+应用扩展快照→显示预览和差异→取消或通过 `Theme::update` 与应用快照原子确认；旧预览结果不得覆盖新选择，失败保留last-good。不得调用 `ThemeRegistry::watch_dir`扫描/监听Zed来源。始终有键盘可达的内置安全主题恢复。

测试真实Rainbow文件（只作遵循许可的测试输入）、light/dark、透明色、低对比、非法色、缺失/未知字段、未来schema、重名/重新导入、源删除、损坏副本、竞态。断言修正后阈值达标、原JSON bytes不变。

## 完成定义

标准Kit组件只消费当前Kit Theme，自定义编辑器/Canvas等只消费同版本应用扩展快照；导入失败不能出现两层版本不一致、半主题或锁死恢复路径。