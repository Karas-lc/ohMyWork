PRAGMA foreign_keys = ON;

CREATE TABLE files (
  id INTEGER PRIMARY KEY,
  resolved_path TEXT NOT NULL UNIQUE,
  volume_key TEXT,
  os_file_id BLOB,
  content_hash BLOB NOT NULL,
  byte_len INTEGER NOT NULL,
  mtime_ticks INTEGER NOT NULL,
  parse_version INTEGER NOT NULL,
  generation INTEGER NOT NULL
);
CREATE TABLE aliases (
  logical_path TEXT PRIMARY KEY,
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  area TEXT NOT NULL CHECK(area IN ('atlas','efforts','archive','other')),
  excluded INTEGER NOT NULL DEFAULT 0 CHECK(excluded IN (0,1))
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
CREATE TABLE index_state (
  key TEXT PRIMARY KEY,
  generation INTEGER NOT NULL
);
INSERT INTO index_state(key, generation) VALUES ('completed_generation', 0);
CREATE VIRTUAL TABLE search_long USING fts5(path_fold, title_fold, body_fold, tokenize='trigram');
CREATE VIRTUAL TABLE search_short USING fts5(grams, tokenize='ascii');
PRAGMA user_version = 1;
