use omw_document::parse;

#[test]
fn unterminated_list_link_before_empty_heading_preserves_source_ranges() {
    let bytes = b"- [ \n\n#";
    let document = parse(bytes).unwrap();
    assert_eq!(document.serialize().as_bytes(), bytes);
    document.validate().unwrap();
}
