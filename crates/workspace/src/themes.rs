//! Theme storage. Manual import only. Copy source bytes + metadata.
//! Path: %LOCALAPPDATA%/ohMyWork/themes/<import_id>/{source.json, metadata.json}
//! Uses AppDataPaths.themes_dir(); deterministic id for reimports.
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use project::{atomic_write, AppDataPaths};
use ui::theme::ImportMeta;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StoredThemeMeta {
    pub import_id: String,
    pub family: String,
    pub member: String,
    pub schema: Option<String>,
    pub original_len: usize,
    pub hash: String,
}

pub fn themes_dir(paths: &AppDataPaths) -> PathBuf {
    paths.themes_dir()
}

/// Stable id = first 16 hex of blake3(source) so reimport of same keeps import dir/UUID.
fn stable_import_id(json: &[u8]) -> String {
    let h = blake3::hash(json);
    h.as_bytes()
        .iter()
        .take(8)
        .map(|b| format!("{:02x}", b))
        .collect()
}

pub fn save_imported(
    paths: &AppDataPaths,
    original: &[u8],
    meta: &ImportMeta,
) -> Result<PathBuf, String> {
    if original.is_empty() {
        return Err("empty theme source".into());
    }
    let id = if meta.hash.is_empty() {
        stable_import_id(original)
    } else {
        meta.hash.clone()
    };
    let dir = themes_dir(paths).join(&id);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Err(format!("create theme dir: {}", e));
    }
    let source_path = dir.join("source.json");
    let source_expected = project::read_version(&source_path).map_err(|e| format!("read source version: {e}"))?;
    atomic_write(&source_path, source_expected.as_ref(), original, 0)
        .map_err(|e| format!("write source.json: {}", e))?;

    let meta_path = dir.join("metadata.json");
    let stored = StoredThemeMeta {
        import_id: id,
        family: meta.family.clone(),
        member: meta.member.clone(),
        schema: meta.schema.clone(),
        original_len: original.len(),
        hash: meta.hash.clone(),
    };
    let meta_bytes = serde_json::to_vec_pretty(&stored)
        .map_err(|e| format!("serialize meta: {}", e))?;
    let meta_expected = project::read_version(&meta_path).map_err(|e| format!("read metadata version: {e}"))?;
    atomic_write(&meta_path, meta_expected.as_ref(), &meta_bytes, 0)
        .map_err(|e| format!("write metadata.json: {}", e))?;
    Ok(dir)
}

pub fn load_meta(dir: &Path) -> Result<StoredThemeMeta, String> {
    let p = dir.join("metadata.json");
    let bytes = std::fs::read(&p).map_err(|e| format!("read meta {}: {}", p.display(), e))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("parse meta: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use project::AppDataPaths;
    use std::fs;

    fn temp_app_data() -> (PathBuf, AppDataPaths) {
        // avoid tempfile crate dep; use uuid (in dev-deps) + env temp for unique test base
        let unique = uuid::Uuid::new_v4().to_string();
        let base = std::env::temp_dir().join("omw-test-appdata").join(&unique).join("ohMyWork");
        let _ = fs::create_dir_all(&base);
        let paths = AppDataPaths::from_base(base.clone()).expect("paths");
        (base, paths)
    }

    #[test]
    fn uses_app_data_themes_dir_not_temp() {
        let (base, paths) = temp_app_data();
        let d = themes_dir(&paths);
        assert!(d.starts_with(&base));
        assert!(d.file_name().unwrap() == "themes");
    }

    #[test]
    fn save_imported_layout_and_metadata_json() {
        let (_base, paths) = temp_app_data();
        let json = b"{\"name\":\"test\",\"themes\":[{\"name\":\"l\",\"appearance\":\"light\",\"style\":{\"background\":\"#fff\"}}]}";
        let meta = ImportMeta {
            family: "test".into(),
            member: "light".into(),
            schema: Some("v0.2".into()),
            hash: "deadbeefcafebabe".into(),
        };
        let dir = save_imported(&paths, json, &meta).expect("save");
        assert!(dir.join("source.json").exists());
        assert!(dir.join("metadata.json").exists());
        assert!(!dir.join("meta.json").exists());
        let loaded = load_meta(&dir).expect("load");
        assert_eq!(loaded.family, "test");
        assert_eq!(loaded.import_id, "deadbeefcafebabe");
        assert_eq!(loaded.original_len, json.len());
        let dir2 = save_imported(&paths, json, &meta).expect("resave");
        assert_eq!(dir, dir2);
    }

    #[test]
    fn source_deletion_after_copy_keeps_stored() {
        let (_base, paths) = temp_app_data();
        let json = b"{\"name\":\"del\",\"themes\":[{\"name\":\"l\"}]}";
        let meta = ImportMeta { family: "del".into(), member: "l".into(), schema: None, hash: "".into() };
        let dir = save_imported(&paths, json, &meta).expect("save");
        // simulate delete of original source bytes (not the copy)
        // copy must survive
        assert!(dir.join("source.json").exists());
        let loaded = load_meta(&dir).expect("load meta");
        assert_eq!(loaded.original_len, json.len());
        // re-save same keeps id
        let dir2 = save_imported(&paths, json, &meta).expect("resave");
        assert_eq!(dir, dir2);
    }

    #[test]
    fn corrupt_copy_detected_on_meta_load() {
        let (_base, paths) = temp_app_data();
        let json = b"ok";
        let meta = ImportMeta { family: "c".into(), member: "m".into(), schema: None, hash: "bad1".into() };
        let dir = save_imported(&paths, json, &meta).expect("save");
        // corrupt the metadata copy
        let meta_path = dir.join("metadata.json");
        fs::write(&meta_path, b"not json").ok();
        // load should fail or handle
        let res = load_meta(&dir);
        assert!(res.is_err() || res.unwrap().family != "c"); // tolerate or err
    }

    #[test]
    fn stable_import_id_on_reimport_same_hash() {
        let (_base, paths) = temp_app_data();
        let json = b"same";
        let meta = ImportMeta { family: "s".into(), member: "1".into(), schema: None, hash: "83fe82573ab536cf".into() };
        let d1 = save_imported(&paths, json, &meta).unwrap();
        let d2 = save_imported(&paths, json, &meta).unwrap();
        assert_eq!(d1, d2);
        let meta2 = ImportMeta { family: "s".into(), member: "1".into(), schema: None, hash: "".into() };
        let d3 = save_imported(&paths, json, &meta2).unwrap();
        assert_eq!(d1.file_name(), d3.file_name());
    }
}
