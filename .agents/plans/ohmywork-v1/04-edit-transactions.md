# 04 — 实现最小编辑事务与序列化

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` 第4节、`docs/adr/adr-0008-visual-editing-save-policy.md`。检查 `omw-document` 已提供 owned lossless tree、byte source ranges、Raw 和逐字节无操作往返测试。

本步只做文档领域逻辑；不接 GPUI、不写磁盘、不完成全部 GFM 结构编辑。

## 产物

- `omw-document/src/{edit,history,serialize}.rs`
- `tests` 中最小改写、Undo/Redo、Unicode、属性测试
- 清晰公开的 `EditTransaction`、视觉文本位置、`TextPatch` API

## 必须实现

1. 语义操作：`InsertText`、`DeleteText`、`ReplaceSelection`、`SplitBlock`、`JoinBlocks`、`ToggleMark(Bold|Italic)`。
2. 事务记录 `NodeId`、语义操作、选择前后值；结构移动保留 ID，删除使用 tombstone。历史不得只存旧行号。
3. 光标/删除按 grapheme；明确 UTF-8 byte 与视觉位置转换。普通输入只作为文本，序列化时转义必要 Markdown 字符。
4. `plan_source_patches(base, working)`：未改节点复用原 `SourceSlice`；只扩大到保证语义正确的最小内联/块容器。
5. 每个 patch 含半开 range、`expected_hash`、replacement；拒绝重叠，按倒序应用到同一基线。
6. 生成 bytes 必须再次解析并验证目标语义；不一致则返回可诊断错误，禁止全文格式化兜底。
7. Raw 内部不可编辑；本步可拒绝跨 Raw 的操作，但必须是类型化错误且不改变状态。

## 测试

- 改一个词不改变相邻段落、空白、换行或未知字段。
- 中文、Emoji、组合字符、HTML实体、反斜线、星号、反引号和多字节选区。
- Split/Join、Bold/Italic 的 Undo/Redo 往返；失败事务保持原状态。
- 至少1000次固定seed随机编辑：范围不越界、树与重解析语义一致、Undo恢复原 bytes。

## 完成定义与交接契约

后续可从公开 API 对文档实体发语义事务并取得最小 patches，不需要直接改 Markdown 字符串。若只能靠整篇重序列化通过测试，本步未完成。报告操作集合、已知明确拒绝边界、测试命令和结果。