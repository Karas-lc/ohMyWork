# ohMyWork

ohMyWork 是面向 Windows 11 x64 的本地个人生产力工作站，主体使用 Rust 与 GPUI，连接用户自行安装和配置的 Pi Agent。

> 当前已接通真实工作台：Workspace 设置、文件树与搜索打开 Markdown、正文显式保存与关闭三分支、任务局部提交、图片/PDF 预览、Homepage、Dock 恢复及 Pi RPC 面板。本轮修复已进行原生交互验证；这不表示 V1 发布验收完成。

## 核心理念

协作不是要求他人改变工作方式来适应 ohMyWork，而是让 ohMyWork 主动兼容现有工具、人员与流程，以尽可能低的摩擦、尽可能快地调动更多资源投入生产。

不建设团队协作系统，不要求账号或必需云服务，不接管用户的 Pi 配置。

## 一个 Workspace，三个分区

ohMyWork 是应用，Workspace 是唯一的本地内容目录，不是每个项目各一个 Workspace。

```text
Workspace/
├── Atlas/      # 概念、资料、人物、共享笔记，多项目共用
├── Efforts/    # 正在投入精力的项目、领域及其专属工作笔记
├── Archive/    # 不再使用、待整理或休眠的内容与 Efforts
└── .ohmywork/
    └── id      # 应用自动创建的稳定身份
```

项目和领域都是 Efforts 下的普通文件夹，无强制项目对象、ID 或模板。Archive 默认不参与搜索和任务汇总，可显式纳入。

## V1 范围

- 单一主窗口，可嵌套横向/纵向分栏，恢复标签与布局。
- CommonMark + GFM 整篇 Markdown 源码编辑；标记始终可见，Raw、HTML 与未知语法可直接编辑。
- 正文 Ctrl+S 保存，任务组件操作立即保存；不恢复崩溃前未保存编辑。
- 基础 Task：checkbox、标题、start、due、priority；无独立 status。
- 文件名与 Markdown 正文搜索，使用专有 `.ohmyworkignore`。
- 无限画布 Homepage，可平移缩放、自由布置内置组件。
- 图片和 PDF 只读预览，其他文件交给系统默认应用。
- Pi 侧边面板：用户选择 cwd、新建或恢复会话，以及主动附加的内容。
- 手动导入 Zed 主题，包括 Rainbow；关键语义色有安全覆盖。
- 简体中文/英文界面，默认快捷键及 `keymap.jsonc` 覆盖。
- 当前用户安装包，无需管理员权限，手动升级。

**V1 发布验证状态**：见 [修复验证记录](docs/verification.md)与[发布检查表](docs/release-checklist.md)。自动回归和本轮原生交互已有实测证据；真实 Pi 提供商、IME/Narrator/多缩放、安装升级卸载、严格 lint 与 GUI 性能预算仍未完成发布验收。

**数据注意事项**：外部文件版本始终优先，检测到外部修改会直接丢弃该文件未保存的本地编辑，不提供恢复。关闭脏文件时仍提供保存/放弃/取消。详见[编辑与保存规则](docs/product-requirements.md)。

## 版本边界

- **V2**：独立进程插件、浏览器/邮箱/微信/飞书连接器、资源信封、自动化与 Dataview 功能级兼容。
- **V3**：日历与 ICS；复杂重复、依赖和时区规则届时重新设计。

V1 不做日历、ICS、RRULE、任务依赖、反向链接或重命名引用更新。

## 本地数据与隐私

默认应用数据统一保存在 `%LOCALAPPDATA%\ohMyWork\`，其中 `settings.json` 保存版本化的唯一 Workspace 根目录和 UUID；使用一个全局 SQLite 索引。Markdown 是内容事实来源，索引可重建。只保留本地日志，用户主动导出诊断包，不做遥测或自动上报。

应用的文件树与索引不是 Pi 的权限边界；内容不会因为被索引就自动发送给 Pi。

## 文档

- [产品定位](PRODUCT.md) · [领域词汇](CONTEXT.md) · [代理协作规则](AGENTS.md)
- [产品需求与性能预算](docs/product-requirements.md)
- [系统架构](docs/architecture.md)
- [Pi 集成](docs/pi-integration.md)
- [基础任务与日历延期](docs/task-calendar.md)
- [主题系统](docs/theme-system.md)
- [路线图](docs/roadmap.md) · [ADR 索引](docs/adr/README.md)

## License

[MIT](LICENSE) © 2026 Karas
