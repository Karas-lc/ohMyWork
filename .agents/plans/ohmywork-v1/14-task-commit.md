# 14 — 实现 Task 局部即时提交

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` SAVE-007..010、`docs/task-calendar.md` 第3/4节、`docs/adr/adr-0008-visual-editing-save-policy.md`。确认安全写队列、外部事件分类、Task精确span和文档语义历史均已通过。

这是高风险边界：组件提交只改目标字段，不能保存整份脏文档。

## 产物

- `omw-document/src/rebase.rs`
- `omw-document/tests/task_commit.rs`
- local写队列的 Task提交请求
- 任务列表/首页可复用的提交状态UI，沿用13的GPUI Kit Checkbox/DatePicker等组件

## Intent 与算法

`TaskIntent { doc_key, baseline_hash, task_anchor, field, value, request_id }`。anchor组合来源范围、结构身份与会话NodeId映射；重复标题不等于同一任务。

1. UI生成 intent并显示 pending，不先确认成功。
2. 若任务只存在于未保存 working、已删除或无法唯一定位，返回可定位错误；禁止保存全文或猜任务。
3. 重读磁盘验证仍等于基线B；若变更，走统一外部优先并取消 intent。
4. 只在B上替换目标字段span；缺字段在任务合适尾部插入；清除只删该字段片段。其他bytes不变，得到B'。
5. 复用07的安全提交。失败不改变working，控件恢复并提示。
6. 成功后在最新W按同NodeId定位，将**目标字段**覆盖为请求值，即使W中该字段有另一个未保存值；标题、其他字段和段落编辑保留。
7. baseline=B'；删除/重写history中涉及该任务该字段的旧逆操作，以B'为基线重放其余语义事务。结构快照恢复时叠加已提交字段。
8. W与B'仍不同则dirty；索引只在commit ack后更新。自身watcher事件不能触发外部重载。

同字段竞争固定为组件操作胜出；不同字段请求按文档写队列串行并重新定位。

## 必测例

磁盘priority=3；W改为2并修改上方段落；组件设9。结果：磁盘仅3→9；W为9且段落仍dirty；撤销段落不能把priority变回2或3。

另测：两字段并发、重复标题、任务移动/删除、新建未保存任务、正在输入/IME、保存失败、外部竞争、Undo结构快照、自写事件、索引更新。

## 完成定义

到达第二检查点：脏正文存在时从列表/组件修改Task，只有目标字段落盘。任何无法安全映射的情况必须失败并保持数据，不允许扩大成全文提交。