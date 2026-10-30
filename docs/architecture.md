# Architecture

> 已确认系统边界与设计约束；以下 Rust crate 划分是当前实现边界，不改变产品范围。以最新需求与 ADR 为准。

## Rust crate 边界

本地 Cargo workspace 恰有五个 package：

| Crate | 职责 | 本地依赖 |
|---|---|---|
| `ui` | 公共主题、对比度与 GPUI 展示集成；不持有产品状态或执行文件 I/O | 无 |
| `project` | 文件系统与安全写盘、扫描/watch、SQLite 索引、Pi 协议/进程 Adapter、预览协议/worker、诊断 | 无 |
| `editor` | 无损 Markdown 与 Task 编辑、`PageDocument`、`DocumentStore`、保存/Task/外部修改流程及编辑器界面 | `ui`、`project` |
| `workspace` | Workspace 会话、查询、Homepage、Pi/预览会话、关闭协调、布局持久化与产品视图 | `ui`、`editor`、`project` |
| `ohmywork` | 应用入口、runtime/assets/keymap 装配及二进制 | `ui`、`editor`、`project`、`workspace` |

依赖方向：`ohmywork → workspace → editor → project`；`ui` 是独立的展示基础层。`project` 不依赖 GPUI。

## 1. 核心模型

```text
一个 ohMyWork 主窗口
├── Homepage：无限画布与内置组件
├── 可嵌套 Pane：Markdown、图片、PDF 标签
└── Pi 侧栏：用户指定 cwd、新建或恢复会话

一个 Workspace
├── Atlas/
├── Efforts/（普通项目/领域目录）
├── Archive/（默认不搜索/汇总任务）
└── .ohmywork/id

%LOCALAPPDATA%/ohMyWork/
├── 持久设置、布局、导入主题、keymap.jsonc
├── 一个全局 SQLite 索引
└── 缓存与本地诊断日志
```

图中的数据分类不是最终磁盘子目录 Schema。Pi 的 cwd 可在 Workspace 外；图不定义 Pi 文件权限。应用无多 Workspace Registry 或项目对象层。

## 2. 文档引擎

Markdown 原文是持久事实来源，也是唯一正文编辑缓冲。

- CommonMark + GFM 用于基础 Task 等语义识别，不限制源码输入。
- Markdown 标记、HTML、Raw、未知语法和未闭合围栏均可作为普通源码编辑；应用不执行 HTML。
- 正文保存直接写入用户编辑的 UTF-8 源内容，不经富文本重序列化，不规范化未改动内容。
- 文档级历史与每个编辑表面的原生选区、滚动和 IME 状态分离。
- 不提供块级可视化模式、双模式、Live Preview 或全文自动格式化。

### 保存状态与事件来源

至少区分磁盘基线、内存编辑、保存中的版本和文件事件来源。

| 操作 | 磁盘结果 | 内存结果 |
|---|---|---|
| 编辑正文 | 不变 | dirty |
| Ctrl+S | 临时文件完整写入后原子替换 | 成功后 clean；失败保留编辑并报告 |
| 关闭脏标签/应用 | 用户选择保存/放弃/取消 | 按选择处理 |
| 真正外部修改 | 已由外部更新 | 无条件重载，彻底丢弃本地未保存编辑 |
| Task 组件修改 | 只提交目标任务变化 | 合并已提交变化，其余脏正文保留 |
| 崩溃/断电 | 保留最近成功磁盘内容 | 未保存编辑丢失，无恢复快照 |

Task 提交不能序列化整个脏文档。以磁盘基线按 Task anchor 构造字段补丁，并逐字节校验旧片段；当前及所有保留历史状态的 `[0, task source_range.end)` 必须与基线一致，否则拒绝写盘，不自动搜索重定位或顺带保存正文。自写 watcher 事件不可触发外部丢弃逻辑。

外部优先不等于允许保存覆盖刚变化的外部数据：保存前须校验磁盘版本，发现变化先采用外部版本。原子替换能力、链接目标/硬链接行为及跨进程竞争需验证，不能宣传绝对无竞争。
当前安全保存实现位于 `project` crate：同目录 create-new 临时文件，写入并 `sync_all` 后在替换前再次校验内容版本；Windows 已有文件使用 `ReplaceFileW`，新文件使用不覆盖移动语义。按 `DocumentKey` 的写队列串行同文件保存，普通请求有界为 256，保存请求走不可丢失队列。Windows 文件身份字段暂不从稳定 Rust 标准库读取，版本校验以 BLAKE3 内容哈希和长度为准。

## 3. 文件服务与索引

- 单一根目录身份在 `.ohmywork/id`，注册自动创建；不为 Effort 分配身份。
- 文件树显示全部未排除文件，编辑限 Markdown；图片/PDF 用只读预览，其他交给系统应用。
- `.ohmyworkignore` 为唯一用户排除规则来源，使用 gitignore 风格语法，不读 `.gitignore`。
- 跟随系统符号链接/Junction，可到根外；检测循环和重复路径，具体去重策略待定。
- Archive 的默认搜索/任务排除是应用查询策略，不是文件删除或 Pi 限制。
- 一个全局 SQLite 建文件名/路径、标题、Markdown 正文及基础 Task 投影；不建反向链接图或全文 PDF/OCR 索引。
- 文件与解析结果带版本/代次；只提交未过期索引结果。
- SQLite 索引可重建；持久设置、布局、手动导入主题必须与可丢弃缓存区分。

当前索引实现将数据库固定在 `%LOCALAPPDATA%/ohMyWork/index.sqlite`，由独占 SQLite 工作线程持有连接。数据库启用 WAL、外键、2 秒 busy timeout 和约 32 MiB cache；schema 迁移或损坏恢复只删除索引文件及其 WAL/共享内存文件，不触碰设置、布局或主题。索引写入以扫描代次为边界，在单事务中替换文件、别名、Task 和两张 FTS；完整扫描结束后才清除消失 alias，过期结果被拒绝。

## 4. App Shell 与 Homepage

一个主窗口、树状嵌套分栏，标签属于 Pane。重启恢复布局、标签、选中项、滚动位置；缺失内容有诊断占位，不恢复脏正文。

Homepage 为逻辑坐标无限画布，支持平移、缩放、自由布置、键盘移动、组件定位和适合全部内容。提供可删除起始布局，不含日历或多 Workspace 组件。

应用内部布局持久状态是事实来源；版本化 JSON 是导入导出边界，不是监听配置文件。V1 不引入插件 UI 宿主。

## 5. 基础任务服务

Task 来自 checkbox、标题以及 start/due 日期和 priority 1–9（9 最高）。没有单独 status、RRULE、依赖、时区或 ICS 映射。

任务汇总与 Task 操作共用文档服务的源范围和版本校验，不另存一份权威任务数据库。日历不是 V1 模块。

## 6. Pi Adapter

PATH/显式路径发现用户 Pi，通过官方 JSONL RPC 启动。用户指定 cwd，选择新建或恢复历史；目录与 Workspace、Effort 身份无强制绑定。

- 只传递用户消息及显式附加：文件路径、选区文本。
- 不自动加入已索引知识，也不改变 Pi 自主读取行为。
- 流式协议事件归约为 UI，不作为执行前审批。
- 不设计宿主自己的 Agent 队列、插话、权限或配置策略。
- 原生 GPUI Extension UI 请求/响应层。
- 用户关闭 Pi 面板时官方 abort 后有序关闭进程；不是暂停功能。
- 视觉收起与显式关闭语义需后续明确，不由实现者擅自解释成后台常驻。

Pi 自身可能加载扩展、写会话或联网，宿主不能因为“不修改配置”就保证运行无副作用。详见 [Pi 集成](pi-integration.md)。

## 7. 主题、快捷键与语言

- 手动导入 Zed 原始 JSON 到 LocalAppData；通过版本适配器生成内部语义 Token。
- 缺失字段回退，关键对比度安全覆盖并给出修正清单；无自动扫描/来源监听。
- 默认键位由 `keymap.jsonc` 覆盖，作用于宿主命令，不修改 Pi keymap。
- 简体中文与英文界面；持久字段/目录名不随 UI 翻译。

## 8. 并发、性能与故障

UI 线程不执行阻塞文件/数据库/进程 I/O、大规模解析或索引。后台工作有界、可取消、携带版本，禁止旧结果覆盖新状态。PDF/图片解码也不阻塞输入，缓存与恶意输入资源限制需定义。

性能预算及本机基线见 [需求文档](product-requirements.md)。首次索引与 Pi/GPU 资源单列，不以主窗口出现冒充启动完成。

单文件错误、主题错误、Pi 退出不应拖垮主应用。派生索引损坏可重建，但不能清除持久布局来冒充恢复。不保存未提交正文恢复快照。

## 9. 分发、隐私与测试

Windows 11 x64 当前用户安装包；无需管理员权限、手动升级，无自动更新。卸载不应删除 Workspace 用户文件；应用数据保留流程待定。

仅本地日志，用户手动导出诊断；默认排除正文、Pi 对话和凭据，无遥测/自动上传。导出中的路径和命令也可能敏感，须定义脱敏。

测试覆盖文档最小写回、Raw、外部版本优先、Task/脏正文合并、自写事件、ignore/Archive、全局索引重建、Pi 假进程、主题、只读预览、双语/IME/键盘、无限画布及性能。

## 10. 尚未冻结的技术方案

编辑引擎/解析器、全局 SQLite Schema、搜索算法、Watcher 实现、Pi 能力探测、keymap 重载、安装签名和测试夹具仍待确定。遵循范围约束；安全保存的实现边界见第2节，不能宣传绝对无竞争。

已锁定，实现不得另选：

- PDF/图片预览：独立 `omw-preview-worker` 进程；`pdfium-render 0.9.4` + PDFium `chromium/7881` 非 V8 动态库；图片 `image 0.25`（png/jpeg/webp/gif 首帧）；帧协议 `u32_le` 头长 + JSON 头 + payload；generation 丢弃过期帧；资源上限与许可证见 `.agents/plans/ohmywork-v1/21-preview-worker.md`。
- 主题关键对比度：WCAG 2.2 相对亮度、sRGB；正文与选中 4.5:1，焦点环 3:1，错误相对背景 4.5:1；禁用相对背景与相邻均 3:1 且不得作为唯一状态；不合格则相对背景或前景二分混合至达标，列出被改 token，不改源 JSON。

