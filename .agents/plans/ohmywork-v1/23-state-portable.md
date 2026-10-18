# 23 — 实现状态恢复与便携模式

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` WS-005/UI-002/HOME-002、`docs/architecture.md` 第1/4节、`docs/adr/adr-0007-single-workspace-storage.md` 和GPUI Kit Dock持久化文档。确认18已有应用包装的 `DockAreaState` schema、Canvas纯数据schema和安全settings写入。

本步恢复持久UI状态和切换便携权威源；绝不恢复未保存正文或Pi对话。

## 数据归属

默认 `%LOCALAPPDATA%/ohMyWork/ui-state.json` 保存版本化Kit `DockAreaState`包装、Panel/Tab引用、选中项、滚动、Canvas、置顶。启用便携后 `.ohmywork/ui-state.json` 是这些可迁移状态的唯一权威源，`.ohmywork/portable.json` 明确模式。

无论便携与否，root绝对路径、Pi命令/会话、日志、全局SQLite、缓存、导入主题副本仍留LocalAppData。便携内容引用使用Workspace相对路径；根外引用不得伪装可迁移。

## 实现顺序

1. 定义版本化App UI state和migration；先校验应用schema、Kit layout version、Panel payload、数字与数量，再把临时 `DockAreaState`交给Dock load。禁止让任意JSON直接创建Panel。
2. 节流保存并使用同目录临时文件+原子替换；退出前flush。禁止保存DocumentSession bytes、history、输入草稿或transcript。
3. 启动先显示壳，再恢复Homepage、Pane、Tab、active、scroll；不自动启动Pi。缺失文件创建诊断placeholder。
4. 启用便携：从当前权威源读取→转换绝对/相对引用→写目标临时文件→重读验证→最后原子更新portable标记；成功后停止写旧源。
5. 禁用同样复制验证后切回LocalAppData。任一步失败保持原权威源和模式，不双写。
6. schema不支持/损坏时保留原文件，显示可操作默认并允许导出诊断。

## 测试

重启恢复Dock分割/Tab/选中/滚动/画布；缺失文件；未知Panel、坏Kit layout version/坏树；保存中断。启用/禁用便携成功与每个故障点回滚；断言任一时刻只有一个权威源。扫描state bytes确认无正文、撤销历史、Pi对话、凭据、本机Pi命令；便携文件不含机器绝对root。

## 完成定义

用户布局可恢复且可迁移，未保存内容仍随进程结束丢失。不能通过双写“提高可靠性”，也不能把索引搬进Workspace。