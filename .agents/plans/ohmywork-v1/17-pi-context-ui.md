# 17 — 实现显式附件与 Extension UI

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/pi-integration.md` 第3–7节、目标Pi版本Extension UI/RPC文档，以及GPUI Kit Input/Textarea/Select/Dialog/Attachment/MessageScroller组件文档。确认16可用fake_pi启动、收流、关闭，且17不会改变启动策略。

目标是第三检查点：用户选cwd/会话，显式发送笔记路径或选区文本，原生显示流式输出与交互。

## 产物

- `omw-pi/src/context.rs` 与完整 client UI request接口
- 用GPUI Kit Sidebar、Textarea、Attachment、Message/MessageScroller构建的Pi输入、附件、预览和消息视图
- 用Kit Dialog、Select、Input、Textarea实现的 `select/confirm/input/editor` 对话层与测试

## 上下文契约

- 待附加项只有用户主动添加；文件项保存原始标签+绝对路径，不由宿主读全文；选区项保存来源标签+当次文本快照。
- 发送前可查看最终包装文本并逐项移除；不同cwd下仍用无歧义绝对路径。
- 不自动附加Workspace目录、最近页、搜索、Task或Archive内容。Archive可被用户主动附加。
- 日志与诊断不记录输入、附件选区或Pi输出正文。

## 实现顺序

1. 输入草稿由长期存在的Kit `TextareaState`持有，附件保存在应用领域状态并由Attachment展示；两者仅驻内存，关闭/重启不持久化对话或草稿。
2. 发送只使用官方prompt命令。若运行中请求因缺少/拒绝 `streamingBehavior` 等官方参数失败，保留草稿并显示原错误；不自动重试成queue/steer/follow_up。
3. 流式事件后台完整消费并保留消息累积值；UI最多每帧取快照，只合并重绘通知，不丢协议消息。
4. `select/confirm/input/editor`按request id组合Kit Select/Dialog/Input/Textarea，复用其焦点、键盘和AccessKit行为；验证输入并准确响应。用户取消、面板关闭、进程退出会结束请求。
5. 未知交互request显示不支持诊断并走协议允许的取消/错误路径。
6. 本计划把“收起/关闭”作为同一个退出入口：走16的 abort/关stdin/等待退出流程；不提供“隐藏但后台继续运行”按钮。切换正文标签不关闭侧栏。若产品以后要区分视觉隐藏与关闭，须先变更现行规格。

## 测试

- 未主动选文件时消息无隐式内容；文件仅传路径；选区传当时文本。
- 附件预览/删除、中文/换行/特殊路径包装。
- 运行中官方拒绝不被改成宿主队列，草稿保留。
- 用Kit `test-support`覆盖四类UI请求的焦点、提交、取消、关闭和并发ID；流式高频事件无丢失。
- 日志扫描无对话、选区、路径正文或凭据。

## 完成定义

Pi面板是协议适配器，不是Agent策略层。不得把工具事件实现成审批拦截，不得承诺abort撤回已发生副作用。