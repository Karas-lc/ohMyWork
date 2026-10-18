# 24 — 完成 keymap、双语与无障碍

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` KEY-001/LANG-001/A11Y-001，以及GPUI Kit [Command](https://gpui-kit.com/zh-CN/component/command.md)、[测试](https://gpui-kit.com/zh-CN/docs/test.md)和[无障碍](https://gpui-kit.com/zh-CN/docs/accessibility.md)文档。确认所有主要界面与命令已经存在，便于做完整清单而非占位。

本步横切命令、语言和可访问性；不增加产品功能。

## 产物

- `omw-app/src/{commands,keymap,i18n}.rs`
- `crates/omw-app/locales/{zh-CN,en}.json`
- 可访问树/键盘/缩放测试及手工清单
- `%LOCALAPPDATA%/ohMyWork/keymap.jsonc`（只在用户主动创建/编辑时存在）

## Keymap

1. 建稳定CommandId全集，所有菜单/按钮/快捷键调用同一命令。
2. 默认键位+受限JSONC `[{context,bindings}]`；用 `jsonc-parser`，context是明确枚举，不是可执行表达式。
3. 用户项覆盖默认，同context后项覆盖前项并诊断；不同context可共存。
4. watcher解析完整临时配置，成功才原子切换last-known-good；坏配置不移除现有键位。避免重复注册。
5. 用Kit `Command`/`CommandState`构建命令面板，条目绑定真实Action，让组件从当前key context显示最终解析键位；禁止手工拼快捷键字符串。始终保留可访问的安全恢复入口，不改Pi keymap。

## 双语

所有宿主文案使用稳定StringId；两份字典key集合完全相同，支持运行时切换。布局容纳中英文扩张。目录名、存储键、Markdown、Pi输出不翻译。

## 无障碍

标准按钮、输入、Tree、表格、DatePicker、Dialog、Dock/Tab、Command优先保留Kit已有的role、状态、焦点和动作，不用自制div覆盖。逐界面补齐领域名称、稳定ID和测试；自定义编辑器、画布、预览图形仍须实现自己的语义、键盘路径和AccessKit动作。所有鼠标功能有键盘路径；焦点可见、状态不只靠颜色；200% Windows缩放不裁关键操作；使用Kit/base减少动态效果偏好。编辑器暴露TextInput/TextRun和选区。

## 验证

坏JSONC、冲突、热重载、实际绑定；zh/en key一致与缺失检测；用Kit `test-support`驱动真实组件的键盘/焦点/状态并断言业务模型；200%缩放、高对比/主题切换、Narrator人工读取、IME和减少动态效果另做目标机验证。无头快照不能证明播报或焦点视觉；任何硬件/人工项未测必须明确列为未通过。

## 完成定义

无鼠标可完成V1核心流程，两种语言无硬编码用户文案，配置错误不使应用失去操作能力。