# 03 — 建立无损 Markdown 文档模型

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` 的 EDIT-001..005、`docs/architecture.md` 第2节、`docs/adr/adr-0008-visual-editing-save-policy.md`。确认里程碑02的 Cargo workspace 能 locked 构建；否则停止。

只实现解析、源范围和无操作字节往返，不实现编辑命令、GPUI、文件保存或索引。

## 产物

创建 `crates/omw-document`：

- `src/{lib,model,parse,source_map,task}.rs`
- `tests/{roundtrip,unicode,raw_nodes}.rs`
- `fixtures/markdown/` 中最小有代表性的 UTF-8/BOM/CRLF/GFM/Raw 夹具

依赖 Comrak `0.55.0`（关闭默认特性，只启用本步所需能力）和 Ropey `1.6` 系列；原始bytes/source map与Rope文本缓冲职责分开。

## 核心契约

- `ByteOffset`/半开 byte range 使用明确新类型；不能把 Unicode 字符、UTF-16单元或 Comrak 行列混用。
- `DocumentSession` 的文档层最少拥有磁盘原始 bytes/text、owned `LosslessTree`、会话稳定 `NodeId` 和 `SourceSlice`。
- 只支持 UTF-8/UTF-8 BOM；其他编码返回只读诊断，不伪装成功。
- 保留 BOM 与 LF/CRLF；无编辑时输出必须逐字节一致。

## 实现顺序

1. 建 `line_start_bytes`，保持 Comrak 默认 UTF-8 byte column，不开启 `sourcepos_chars`。
2. 用 `parse_document`，仅启用 table、tasklist、strikethrough、autolink；关闭 footnote/wiki/frontmatter 等扩展。
3. 把 Arena AST 转为不含借用生命周期的 owned tree 后释放 Arena。
4. 谨慎把 1-based sourcepos 转半开范围；空/生成/不可靠节点显式标记，禁止机械给所有 end 加1。
5. 容器记录子节点间隙、分隔符、缩进、换行和原始子片段，使每个源 byte 都有归属。
6. 原始 HTML、未知或无法安全解释的范围建 Raw 节点；不执行、不改内部。
7. 记录 task checkbox 的 symbol source range；任务元数据只在语义文本中识别，不扫描代码。

## 测试与完成定义

`cargo test -p omw-document --test roundtrip` 至少覆盖：BOM、LF/CRLF、中文/Emoji、多字节边界、嵌套列表、表格、代码、原始HTML、未知片段、Task。断言无操作序列化 bytes 完全相同，且所有范围有效、不重叠越界。

后续只可依赖公开类型和无损解析契约，不可依赖 Comrak Arena。若任一夹具无法建立可信源归属，停止并修模型，不能把全文 formatter 当兜底。