# 11 — 建立全局 SQLite 索引

## 执行边界与前置检查

先读 `AGENTS.md`、`docs/product-requirements.md` WS-006/SEARCH、`docs/architecture.md` 第3/8节。确认扫描器能提供 logical alias、resolved path、排除状态和内容代次；文档解析器能投影标题/正文/Task。

本步建立索引存储与 ingest，不做最终搜索 UI/中文查询（12）。

## 产物

- `omw-local/src/index/{mod,db,migrate,ingest}.rs`
- `omw-local/migrations/001_index.sql`
- `omw-local/tests/index.rs`
- `%LOCALAPPDATA%/ohMyWork/index.sqlite`，全应用唯一

## 数据契约

迁移必须建立以下字段和约束（可为迁移框架调整排版，不可改变语义）：

```sql
CREATE TABLE files (
  id INTEGER PRIMARY KEY,
  resolved_path TEXT NOT NULL UNIQUE,
  volume_key TEXT, os_file_id BLOB,
  content_hash BLOB NOT NULL, byte_len INTEGER NOT NULL,
  mtime_ticks INTEGER NOT NULL, parse_version INTEGER NOT NULL,
  generation INTEGER NOT NULL
);
CREATE TABLE aliases (
  logical_path TEXT PRIMARY KEY,
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  area TEXT NOT NULL CHECK(area IN ('atlas','efforts','archive','other')),
  excluded INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX aliases_file ON aliases(file_id);
CREATE TABLE tasks (
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  ordinal INTEGER NOT NULL, source_start INTEGER NOT NULL, source_end INTEGER NOT NULL,
  anchor_hash BLOB NOT NULL, title TEXT NOT NULL,
  completed INTEGER NOT NULL CHECK(completed IN (0,1)),
  start_date TEXT, due_date TEXT,
  priority INTEGER CHECK(priority BETWEEN 1 AND 9),
  diagnostics_json TEXT NOT NULL DEFAULT '[]',
  PRIMARY KEY(file_id, ordinal)
);
CREATE VIRTUAL TABLE search_long USING fts5(path_fold, title_fold, body_fold, tokenize='trigram');
CREATE VIRTUAL TABLE search_short USING fts5(grams, tokenize='ascii');
PRAGMA user_version = 1;
```

两张 FTS 的 rowid 等于 `files.id`。使用普通 FTS而非 contentless；Task/FTS 由同一事务显式替换，不能期待外键清理虚表。

`files.id` 仅是可重建索引内部 ID；不写回 Markdown。resolved path 去重，aliases 保存 logical path、area和excluded。原子替换后按 resolved path 延续记录并更新 OS identity。

## 实现顺序

1. 用 `rusqlite 0.40.2` bundled SQLite；单独线程独占连接，WAL、foreign_keys、`busy_timeout=2000ms`、约32MiB cache。
2. 版本化 migration；schema不兼容/损坏时仅重建索引，绝不删除 settings/ui-state/themes。
3. 管线：scan→stat/read/hash→parse snapshot→projection→单事务替换 files/aliases/tasks/FTS。ingest 通过窄类型接收已正规化长文本与短token投影；本步可用固定测试投影证明事务，NFKC/uni/bi生成只在12实现，禁止另选搜索算法。
4. 扫描用 `scan_generation`；只有完整成功结束才删除消失 alias，失败扫描不能清全库。
5. 单文件按 hash 跳过重复解析；提交前核对 generation，旧后台结果不能覆盖新文件。
6. 文件读完即释放；批次短事务；首次扫描不阻塞 UI。

## 测试

- migration/重开、WAL设置、重复事件幂等、同目标多alias、Archive area。
- Task与两FTS在故障注入时全有或全无；删除文件显式清虚表。
- 扫描中断不删既有记录；旧generation被拒绝；损坏DB重建且持久设置不变。
- 大文件逐个释放，不把语料整体驻留。

## 完成定义

SQLite 只是磁盘事实的派生投影，可完整重建。UI启动无需等待扫描。完成后12可通过专用线程请求查询，但不能直接跨线程共享连接。