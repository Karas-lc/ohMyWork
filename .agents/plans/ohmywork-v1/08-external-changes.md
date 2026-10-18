# 08 — 处理 watcher 与外部版本优先

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` SAVE-004/005/009、`docs/architecture.md` 第2节、`docs/adr/adr-0008-visual-editing-save-policy.md`。确认07的 `DiskVersion`、串行写队列和 `CommitAck` 已存在且故障测试通过。

本步只解决文件事件分类和文档重载；不做全 Workspace 扫描/索引。

## 产物

- `omw-local/src/{watch,identity}.rs`
- `DocumentSession` 外部事件 reducer
- `omw-local/tests/watcher.rs` 与确定性交错测试

## 固定行为

- watcher 事件只表示“路径可能变化”，永远先重读真实内容。
- 真正外部内容变化立即替换 baseline/working、清 dirty和该文档全部 Undo/Redo；不询问、不保存恢复副本。
- 自身成功提交不得覆盖保存期间产生的后续 working 编辑。

## 实现顺序

1. 用 `notify` 接收事件，转成按规范 `DocumentKey` 合并的有界脏集合；队列满时合并/背压，不静默丢路径。
2. 临时文件使用应用前缀并可过滤该路径，但禁止用时间窗屏蔽整个目录。
3. 重读路径并计算 hash/文件身份，与最近已确认和进行中的 commit序号/hash 比对。
4. 内容相同的重复/元数据事件只刷新身份；命中 `CommitAck` 只更新 baseline/索引通知，不以磁盘覆盖 working。
5. 不匹配的实际内容执行外部 reload：新代次、新解析树、清历史；外部删除/替换进入明确状态。
6. 激活文档与保存前都主动重读校验，保证事件丢失不破坏政策。
7. 后台结果带 request/doc revision；旧代次结果不可回写。

## 测试

- 脏文件被外部改写后立即采用磁盘，Undo无法恢复丢弃内容。
- 保存期间继续输入 + 自写 watcher 回声，后续输入仍 dirty。
- 重复事件、乱序、丢失、临时替换、外部删除/重建、原子替换导致 File ID 变化。
- 事件风暴有界且最终每个实际变化路径会被核对。

## 完成定义

文件变化来源按内容与提交身份判断，不按“最近若干毫秒”猜测。若无法确认来源，非匹配内容按真正外部变化处理。完成报告必须明确执行过会导致本地未保存内容丢失的测试。