//! 无操作字节往返 + 范围有效性。对每个夹具断言：
//! - `serialize()` 与输入 bytes 逐字节一致（含 BOM/CRLF）；
//! - 树不变量校验通过（所有范围有效、不重叠、不越界、Document 覆盖全文）。

use omw_document::parse::{ParseError, parse};
use omw_document::task::task_items;
use omw_document::{DocumentSession, LosslessTree, NodeFlags, NodeKind, SourceEncoding};
use std::path::PathBuf;

fn fixture(name: &str) -> Vec<u8> {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "fixtures", "markdown", name]
        .iter()
        .collect();
    std::fs::read(path).unwrap()
}

fn assert_roundtrip_and_valid(bytes: &[u8]) -> DocumentSession {
    let s = parse(bytes).expect("parse should succeed");
    assert_eq!(
        s.serialize().as_bytes(),
        bytes,
        "serialize must be byte-identical"
    );
    if let Err(issues) = s.validate() {
        panic!("tree validation failed: {issues:#?}");
    }
    s
}

#[test]
fn utf8_fixture_roundtrips() {
    let bytes = fixture("utf8.md");
    let s = assert_roundtrip_and_valid(&bytes);
    assert_eq!(s.encoding(), SourceEncoding::Utf8);
    // 代表性结构：根 Document 存在，且至少包含标题/列表/表格/代码。
    let kinds_present = kinds_present(s.tree());
    assert!(kinds_present.contains(&"Heading".to_string()));
    assert!(kinds_present.contains(&"List".to_string()));
    assert!(kinds_present.contains(&"CodeBlock".to_string()));
    assert!(s.tree().len() > 5);
}

#[test]
fn gfm_fixture_roundtrips() {
    let bytes = fixture("gfm.md");
    let s = assert_roundtrip_and_valid(&bytes);
    let kinds_present = kinds_present(s.tree());
    assert!(kinds_present.contains(&"Table".to_string()));
    assert!(kinds_present.contains(&"Strikethrough".to_string()));
    assert!(kinds_present.contains(&"TaskItem".to_string()));
    assert_eq!(task_items(s.tree()).len(), 2, "两个任务项（含代码块外的）");
}

#[test]
fn bom_crlf_fixture_roundtrips() {
    let bytes = fixture("bom-crlf.md");
    assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
    let s = assert_roundtrip_and_valid(&bytes);
    assert_eq!(s.encoding(), SourceEncoding::Utf8WithBom);
    // BOM 字节归属 Document 的前导 gap：首个子块应从偏移 3 开始。
    let root = s.tree().root();
    let first_child = s.tree().node(root).children.first().copied().unwrap();
    let r = s.source_range(first_child).unwrap();
    assert_eq!(r.start.to_usize(), 3, "首个块必须从 BOM 之后开始");
}

#[test]
fn raw_fixture_roundtrips() {
    let bytes = fixture("raw.md");
    let s = assert_roundtrip_and_valid(&bytes);
    let kinds_present = kinds_present(s.tree());
    assert!(kinds_present.contains(&"HtmlBlock".to_string()));
    assert!(kinds_present.contains(&"HtmlInline".to_string()));
    // 注释后的脚本/内联内容原样保留（由 serialize 相等性覆盖）。
}

#[test]
fn empty_document() {
    let s = assert_roundtrip_and_valid(b"");
    assert_eq!(s.encoding(), SourceEncoding::Utf8);
    let root = s.tree().root();
    assert_eq!(s.tree().node(root).kind, NodeKind::Document);
    assert_eq!(s.source_range(root), None, "空文档 Document 不可归属");
    assert!(
        s.tree().node(root).flags.contains(NodeFlags::UNATTRIBUTED),
        "空文档标记 UNATTRIBUTED"
    );
}

#[test]
fn whitespace_only_document() {
    let bytes = fixture("edge-whitespace.md");
    let s = assert_roundtrip_and_valid(&bytes);
    assert!(s.tree().node(s.tree().root()).children.is_empty());
}

#[test]
fn empty_fixture_file() {
    let bytes = fixture("edge-empty.md");
    assert!(bytes.is_empty());
    let s = assert_roundtrip_and_valid(&bytes);
    assert!(s.tree().node(s.tree().root()).children.is_empty());
}

#[test]
fn unsupported_encoding_is_readonly_diagnostic() {
    // UTF-16LE BOM。
    let bytes = [0xFF, 0xFE, b'h', 0x00, b'i', 0x00];
    let err = match parse(&bytes) {
        Err(e) => e,
        Ok(_) => panic!("expected UnsupportedEncoding"),
    };
    assert!(matches!(err, ParseError::UnsupportedEncoding { .. }));
    // 无效 UTF-8。
    let bytes = b"ab\xffcd";
    let err = match parse(bytes) {
        Err(e) => e,
        Ok(_) => panic!("expected InvalidUtf8"),
    };
    assert!(matches!(err, ParseError::InvalidUtf8 { position: 2 }));
}

#[test]
fn node_slices_are_contiguous_and_in_bounds() {
    let bytes = fixture("utf8.md");
    let s = assert_roundtrip_and_valid(&bytes);
    for id in s.tree().iter() {
        if let Some(sl) = s.source_slice(id) {
            assert_eq!(sl.range.to_byte_range().len(), sl.text.len());
            assert!(sl.range.end.to_usize() <= bytes.len());
        }
    }
}

#[test]
fn every_byte_belongs_somewhere() {
    // 容器威力校验：Document 的子节点+间隙必须完整拼合 [0, len)。
    let bytes = fixture("gfm.md");
    let s = assert_roundtrip_and_valid(&bytes);
    let root = s.tree().root();
    let rn = s.tree().node(root);
    assert_eq!(rn.source.unwrap().to_byte_range(), 0..bytes.len());
    assert_eq!(rn.gaps.len(), rn.children.len() + 1);
    let mut covered = 0usize;
    for i in 0..rn.children.len() {
        covered += s.tree().node(rn.children[i]).source.unwrap().len() as usize;
        covered += rn.gaps[i].len() as usize;
    }
    covered += rn.gaps[rn.gaps.len() - 1].len() as usize;
    assert_eq!(covered, bytes.len(), "Document 子字节+间隙必须覆盖全文");
}

fn kinds_present(tree: &LosslessTree) -> Vec<String> {
    tree.nodes()
        .iter()
        .map(|n| {
            format!("{:?}", n.kind)
                .split([' ', '('])
                .next()
                .unwrap()
                .to_string()
        })
        .collect()
}
