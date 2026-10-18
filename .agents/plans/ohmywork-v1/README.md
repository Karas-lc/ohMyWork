# ohMyWork V1 隔离执行计划索引

> 状态：待审核。来源为 `../2026-10-02T07-21-54-001Z-ohmywork-v1.md`。本目录把总方案拆成 28 个可独立投递给下级模型的里程碑；不表示任何功能已经实现或验证。

## 调度规则

1. 严格按 01→28 执行，一次只把**当前文件**交给实现 Agent；不要把上一步对话、总结或总方案塞进下一步上下文。
2. 每个 Agent 从仓库当前状态开始，先读根 `AGENTS.md` 和当前计划列出的少量规格，再检查前置产物。它不得假定上一 Agent 说过什么。
3. 前置产物缺失、接口与当前计划不符或现行规格冲突时，停止并报告；不要在当前里程碑内偷偷补做多个前序里程碑。
4. 总方案仍是设计来源，但执行 Agent 默认**不要读取总方案**；当前计划已摘录本步所需决策。只有当前计划明确遗漏且现行规格无法消歧时，才回查总方案。
5. 每步只实现当前范围。先增加失败测试/夹具，再做最小实现，再跑当前测试和既有回归。
6. 不自动提交，不覆盖用户未提交改动。每步结束按计划中的“完成报告”交接；下一 Agent只依赖仓库产物和测试，不依赖该报告进入上下文。
7. 这些计划批准后，先确保长期规格已同步采用总方案中的具体默认值。若 `docs/` 仍把本步关键选择写成“待定”，先由上级协调更新规格，不能让实现 Agent自行选范围。

## 固定产品边界

- Windows 11 x64、Rust/GPUI Kit（精确锁定0.7.0；底层GPUI由Kit配套锁定）、一个 Workspace；Atlas/Efforts/Archive 是真实目录。
- Markdown 是事实来源；纯可视化 CommonMark + GFM，无源码模式/Live Preview。未知内容以 Raw 原样保留。
- 正文 Ctrl+S；真正外部修改无条件优先并丢弃未保存编辑；Task 组件只立即提交任务范围。
- 一个全局 SQLite 派生索引；只读图片/PDF 预览；Pi 仅适配用户官方 RPC，不改其配置。
- V1 无多 Workspace、项目对象、日历/ICS、重复/依赖、反向链接、自动改链接、插件系统或自动更新。

## 里程碑

1. [固定开发环境并实现 doctor](01-environment-doctor.md)
2. [创建最小 Cargo/GPUI Kit 应用](02-bootstrap-gpui-kit.md)
3. [建立无损 Markdown 文档模型](03-lossless-document.md)
4. [实现最小编辑事务与序列化](04-edit-transactions.md)
5. [接入 GPUI Kit 与自定义段落编辑器](05-gpui-kit-editor.md)
6. [实现单 Workspace 与 LocalAppData](06-workspace-state.md)
7. [实现安全 Ctrl+S 与关闭行为](07-safe-save.md)
8. [处理 watcher 与外部版本优先](08-external-changes.md)
9. [完成 GFM 可视化编辑](09-gfm-editor.md)
10. [实现文件树、ignore 与链接目录](10-file-tree.md)
11. [建立全局 SQLite 索引](11-sqlite-index.md)
12. [实现中文短词搜索](12-search.md)
13. [实现基础 Task 解析与列表](13-task-list.md)
14. [实现 Task 局部即时提交](14-task-commit.md)
15. [实现 Pi 协议纯逻辑与假进程](15-pi-protocol.md)
16. [实现 Windows Pi 启动与会话](16-pi-launch.md)
17. [实现显式附件与 Extension UI](17-pi-context-ui.md)
18. [实现 Pane 树与标签](18-panes.md)
19. [实现 Homepage 画布数学与交互](19-canvas.md)
20. [实现 Homepage 组件与布局保存](20-home-widgets.md)
21. [实现独立图片/PDF 预览进程](21-preview-worker.md)
22. [实现主题导入与安全覆盖](22-theme.md)
23. [实现状态恢复与便携模式](23-state-portable.md)
24. [完成 keymap、双语与无障碍](24-keymap-i18n-a11y.md)
25. [实现诊断与隐私保护](25-diagnostics.md)
26. [建立可复现性能验证并优化](26-performance.md)
27. [制作当前用户安装器与手动升级](27-installer.md)
28. [执行 V1 发布验收与文档收口](28-release-acceptance.md)

## 检查点

- 完成 09：可选目录、打开笔记、纯可视化编辑、Ctrl+S、外部修改刷新。
- 完成 14：脏正文存在时，Task 组件只提交目标字段。
- 完成 17：用户选择任意 cwd/会话，显式发送路径或选区，原生展示 Pi 输出与交互。
- 完成 28：V1 需求、安装、性能、隐私、无障碍证据闭环。
