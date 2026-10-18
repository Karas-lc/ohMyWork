# 07 — 实现安全 Ctrl+S 与关闭行为

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` SAVE-001..006、`docs/architecture.md` 第2节、`docs/adr/adr-0008-visual-editing-save-policy.md`。确认文档层能产生基于磁盘 baseline 的最小 patch，Workspace/路径服务可用。

本步实现正文显式保存和脏内容关闭；watcher、自写事件去重和 Task 局部提交分别留到08、14。

## 产物

- `omw-local/src/{atomic_write,write_queue}.rs`
- `DocumentSession` 保存状态 reducer 与 app 保存/关闭命令
- `omw-local/tests/files.rs` 的故障注入测试

文件I/O通过后台 `LocalService`，普通请求队列初始容量256；满时合并可合并读取或施加背压，保存请求绝不能丢。UI线程禁止文件I/O或等待同步锁。

## 保存状态与算法

1. 打开文件记录 `DiskSnapshot { bytes/text/tree, DiskVersion { blake3 hash,len,file_identity? } }`；working/revision/history 独立。
2. Ctrl+S 在 UI 取当前 revision 和序列化 bytes/patch 快照，置 `Saving(revision)` 后立即返回；输入仍可继续。
3. 后台按 `DocumentKey` 串行，队列有界且保存请求不得丢失。重读磁盘核对 baseline hash；不同时返回 `ExternalChanged`，不得写盘。
4. 相同时在同目录 create-new 应用专用临时文件，完整写入、`FlushFileBuffers`、关闭写句柄，再核对源版本。
5. 已有文件用 Windows `ReplaceFileW`；新文件使用不覆盖未知目标的移动语义。失败保留原文件、清理临时文件，禁止退化为原地 overwrite。
6. 提交后重读身份/hash，返回含请求revision、旧/新hash、写入bytes的 `CommitAck`。
7. UI revision 未变则 clean；若保存期间继续编辑，只更新磁盘 baseline并按语义事务重基，后续编辑保持 dirty。
8. 关闭脏标签/应用使用GPUI Kit Dialog/Button呈现保存、放弃、取消；领域reducer拥有决策，不让Dialog持有第二份状态。不写恢复草稿，多文档流程中取消不得误关。

## 验证

- 保存成功、保存中继续输入、重复保存按文件串行、不同文件可后台处理。
- 注入磁盘满、锁定、flush/replace失败、第二次版本检查变化；旧目标完整，无半文件/遗留临时文件。
- 检测外变时返回事件给 reducer，不覆盖磁盘。
- 关闭三分支和批量关闭；搜索 LocalAppData 确认无未保存正文快照。

## 完成定义

Ctrl+S 有安全原子提交路径和清晰状态，但不要声称消除所有跨进程 TOCTOU。无法实现 Windows替换语义时停止并提供最小复现，不以普通 `fs::write` 通过。