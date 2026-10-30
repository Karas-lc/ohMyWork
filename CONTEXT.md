# ohMyWork Domain Context

统一产品词汇；实现细节见 `docs/`。

## Language

**ohMyWork**：本地个人工作站应用，而不是其中的项目或 Workspace。

**Workspace**：应用使用的唯一内容根目录，包含 Atlas、Efforts 与 Archive。
_Avoid_: 将每个项目称为 Workspace 或 Vault。

**Atlas**：脱离时间、多项目共用的知识，包括概念、资料、人物和共享笔记。

**Efforts**：当前投入精力的项目、领域及其专属工作笔记所在分区。

**Effort**：Efforts 中一个用普通文件夹组织的项目或领域。
_Avoid_: 默认具备项目 ID、状态或模板的专门对象。

**Archive**：不再使用、待整理或休眠的内容及 Efforts 所在分区，不等于回收站。

**Page**：本地 Markdown 文件承载的页面。

**Raw 内容**：Markdown 源码中任何未知或不需解析的文本；源码编辑器将其作为普通文本处理，不执行 HTML。
**Task**：Markdown checkbox 承载、可附加开始日期、截止日期与优先级的工作项。

**Homepage**：应用级行动主页，用户在无限画布上布置内置组件。

**Pi 工作目录**：用户为 Pi 选择的执行上下文目录，不必等于 Workspace 或某个 Effort。

**Pi Session**：由 Pi 管理、由用户选择新建或恢复的对话历史。

**附加上下文**：用户明确选入待发送清单、准备随消息交给 Pi 的文件路径或文本。

**便携模式**：使可迁移应用状态随 Workspace 目录保存的可选模式。

**Resource**：未来资源系统中具有全局身份、来源与原始载荷的内容对象。

**Connector**：未来连接外部产品、聚合资源并提供动作的集成能力。

## Relationships

- 一个 **Workspace** 包含 **Atlas**、**Efforts**、**Archive** 三个真实目录。
- **Efforts** 可包含多个 **Effort**；**Atlas** 的知识可供多个 Effort 共用。
- **Page** 可包含多个 **Task**。
- **Pi 工作目录** 不由分区位置自动决定，也不构成文件访问权限边界。
- **附加上下文** 由用户选择，不由搜索或索引自动生成。

## Example dialogue

> **用户：**这个项目要新建 Workspace 吗？
>
> **开发者：**不用，在 Efforts 中建普通文件夹即可。共享资料放 Atlas，休眠后可移到 Archive。
>
> **用户：**这些资料会自动进 Pi 吗？
>
> **开发者：**不会。你决定应用附加什么，Pi 自己的配置和工具行为仍由 Pi 负责。
