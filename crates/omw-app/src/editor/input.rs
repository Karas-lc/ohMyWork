use omw_document::{ByteOffset, EditSession, VisualPos, VisualRange};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Utf16Range {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputError {
    InvalidUtf16Boundary,
    OutOfBounds,
    Edit(String),
}

impl std::fmt::Display for InputError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUtf16Boundary => {
                formatter.write_str("The text range splits a Unicode character.")
            }
            Self::OutOfBounds => {
                formatter.write_str("The text range is outside the visible document.")
            }
            Self::Edit(message) => formatter.write_str(message),
        }
    }
}
impl std::error::Error for InputError {}

#[derive(Clone, Debug)]
pub struct VisibleProjection {
    pub text: String,
    /// Source visual boundaries, one for each visible grapheme boundary.
    pub source_boundaries: Vec<VisualPos>,
    source_byte_boundaries: Vec<usize>,
}

impl VisibleProjection {
    pub fn from_session(session: &EditSession) -> Self {
        let mut text = String::new();
        let mut boundaries = Vec::new();
        let mut previous_end: Option<u32> = None;
        let source = session.text().to_string();
        // Build source coordinates once, not once per visible grapheme.
        let source_starts: Vec<_> = source
            .grapheme_indices(true)
            .map(|(byte, _)| byte)
            .chain(std::iter::once(source.len()))
            .collect();
        let position = |byte: usize| {
            source_starts
                .binary_search(&byte)
                .ok()
                .and_then(|index| u32::try_from(index).ok())
                .map(VisualPos)
        };
        for (_, bytes) in session.text_leaves() {
            if let Some(previous) = previous_end {
                let start = bytes.start.0 as usize;
                let end = start.min(source.len());
                if previous as usize <= end
                    && source.as_bytes()[previous as usize..end].contains(&b'\n')
                    && !boundaries.is_empty()
                {
                    text.push('\n');
                    if let Some(position) = position(bytes.start.to_usize()) {
                        boundaries.push(position);
                    }
                }
            }
            let Some(chunk) = session.byte_slice(bytes) else {
                continue;
            };
            if boundaries.is_empty()
                && let Some(pos) = position(bytes.start.to_usize())
            {
                boundaries.push(pos);
            }
            previous_end = Some(bytes.end.0);
            let graphemes: Vec<(usize, &str)> = chunk.grapheme_indices(true).collect();
            let mut index = 0;
            while index < graphemes.len() {
                let (byte, grapheme) = graphemes[index];
                if grapheme == "\\"
                    && index + 1 < graphemes.len()
                    && is_escapable(graphemes[index + 1].1)
                {
                    let (next_byte, escaped) = graphemes[index + 1];
                    text.push_str(escaped);
                    let source_byte = bytes.start.0 as usize + next_byte + escaped.len();
                    if let Some(pos) = position(source_byte) {
                        boundaries.push(pos);
                    }
                    index += 2;
                    continue;
                }
                text.push_str(grapheme);
                let source_byte = bytes.start.0 as usize + byte + grapheme.len();
                if let Some(pos) = position(source_byte) {
                    boundaries.push(pos);
                }
                index += 1;
            }
        }
        if boundaries.is_empty() {
            boundaries.push(VisualPos(0));
        }
        Self {
            text,
            source_boundaries: boundaries,
            source_byte_boundaries: source_starts,
        }
    }

    pub(crate) fn source_position(&self, byte: ByteOffset) -> Option<VisualPos> {
        self.source_byte_boundaries
            .binary_search(&byte.to_usize())
            .ok()
            .and_then(|index| u32::try_from(index).ok())
            .map(VisualPos)
    }

    pub fn source_range(&self, range: VisualRange) -> Result<VisualRange, InputError> {
        let start = self
            .source_boundaries
            .get(range.start.to_usize())
            .copied()
            .ok_or(InputError::OutOfBounds)?;
        let end = self
            .source_boundaries
            .get(range.end.to_usize())
            .copied()
            .ok_or(InputError::OutOfBounds)?;
        Ok(VisualRange::new(start, end))
    }
}

fn is_escapable(grapheme: &str) -> bool {
    grapheme.len() == 1 && "!\\\"#$%&'()*+,-./:;<=>?@[\\\\]^_`{|}~".contains(grapheme)
}

/// Translate a platform UTF-16 range to grapheme-based visible positions.
/// The caller supplies visible text only; Markdown delimiters must not be included.
pub fn utf16_range_to_visual(text: &str, range: Utf16Range) -> Result<VisualRange, InputError> {
    if range.start > range.end {
        return Err(InputError::OutOfBounds);
    }
    let mut utf16 = 0usize;
    let mut starts = vec![0usize];
    for (byte, grapheme) in text.grapheme_indices(true) {
        let _ = byte;
        utf16 += grapheme.encode_utf16().count();
        starts.push(utf16);
    }
    let start = starts
        .iter()
        .position(|&n| n == range.start)
        .ok_or(InputError::InvalidUtf16Boundary)?;
    let end = starts
        .iter()
        .position(|&n| n == range.end)
        .ok_or(InputError::InvalidUtf16Boundary)?;
    Ok(VisualRange::new(
        VisualPos(start as u32),
        VisualPos(end as u32),
    ))
}

/// Delete visible text, joining paragraph blocks when a selection crosses them.
pub fn delete_visible_range(
    session: &mut EditSession,
    projection: &VisibleProjection,
    range: VisualRange,
) -> Result<(), InputError> {
    let source = projection.source_range(range)?;
    let removes_only_separator = range.end.0 == range.start.0 + 1
        && projection.text.graphemes(true).nth(range.start.to_usize()) == Some("\n");
    if removes_only_separator {
        session
            .join_blocks(source.start)
            .map(|_| ())
            .map_err(|error| InputError::Edit(error.to_string()))
    } else {
        let history_before = session.history().undo_depth();
        if let Err(error) = join_selection_blocks(session, source) {
            while session.history().undo_depth() > history_before {
                if session.undo().is_err() {
                    break;
                }
            }
            return session
                .delete_range(source)
                .map(|_| ())
                .map_err(|fallback| InputError::Edit(format!("{error}; fallback: {fallback}")));
        }
        let projection = VisibleProjection::from_session(session);
        let source = projection.source_range(range)?;
        session
            .delete_text(source)
            .map(|_| ())
            .map_err(|error| InputError::Edit(error.to_string()))
    }
}

fn join_selection_blocks(session: &mut EditSession, source: VisualRange) -> Result<(), InputError> {
    let start_byte = session
        .visual_to_byte(source.start)
        .ok_or(InputError::OutOfBounds)?;
    let mut end_byte = session
        .visual_to_byte(source.end)
        .ok_or(InputError::OutOfBounds)?;
    loop {
        let tree = session.semantic_tree();
        let start_paragraph = tree.iter().find_map(|id| {
            let node = tree.node(id);
            (node.kind == omw_document::NodeKind::Paragraph)
                .then_some(node.source)
                .flatten()
                .filter(|range| {
                    range.start <= start_byte
                        && (start_byte < range.end || start_byte == range.start)
                })
        });
        let end_paragraph = tree.iter().find_map(|id| {
            let node = tree.node(id);
            (node.kind == omw_document::NodeKind::Paragraph)
                .then_some(node.source)
                .flatten()
                .filter(|range| range.start < end_byte && end_byte <= range.end)
        });
        let Some(start_paragraph) = start_paragraph else {
            return Err(InputError::Edit(
                "selection does not start in an editable paragraph".into(),
            ));
        };
        let Some(end_paragraph) = end_paragraph else {
            return Err(InputError::Edit(
                "selection does not end in an editable paragraph".into(),
            ));
        };
        if start_paragraph == end_paragraph {
            return Ok(());
        }
        let next_start = tree
            .iter()
            .filter_map(|id| {
                let node = tree.node(id);
                (node.kind == omw_document::NodeKind::Paragraph)
                    .then_some(node.source)
                    .flatten()
                    .filter(|range| range.start > start_paragraph.end)
                    .map(|range| range.start)
            })
            .min()
            .ok_or_else(|| InputError::Edit("selection has no joinable paragraph".into()))?;
        let join_at = session
            .byte_to_visual(start_paragraph.end)
            .ok_or(InputError::OutOfBounds)?;
        session
            .join_blocks(join_at)
            .map_err(|error| InputError::Edit(error.to_string()))?;
        let delta = 1i64 - i64::from(next_start.0 - start_paragraph.end.0);
        if end_byte.0 >= next_start.0 {
            end_byte = omw_document::ByteOffset(
                u32::try_from(i64::from(end_byte.0) + delta)
                    .map_err(|_| InputError::OutOfBounds)?,
            );
        }
    }
}

/// Apply committed plain-text input to the domain edit transaction; IME marked text stays view-local.
pub fn commit_text(
    session: &mut EditSession,
    range: VisualRange,
    text: &str,
) -> Result<(), InputError> {
    if !range.is_empty() {
        let projection = VisibleProjection::from_session(session);
        let source = projection.source_range(range)?;
        join_selection_blocks(session, source)?;
    }
    let projection = VisibleProjection::from_session(session);
    session
        .replace_selection(projection.source_range(range)?, text)
        .map(|_| ())
        .map_err(|e| InputError::Edit(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_omits_markdown_delimiters_and_maps_visible_text() {
        let document = omw_document::parse(b"**hello**").unwrap();
        let session = EditSession::from_document(&document);
        let projection = VisibleProjection::from_session(&session);
        assert_eq!(projection.text, "hello");
        assert_eq!(projection.source_boundaries.len(), 6);
        assert_eq!(projection.source_boundaries[0].0, 2);
    }

    #[test]
    fn projection_preserves_visible_paragraph_boundaries() {
        let document = omw_document::parse(b"first\n\nsecond").unwrap();
        let session = EditSession::from_document(&document);
        assert_eq!(
            VisibleProjection::from_session(&session).text,
            "first\nsecond"
        );
    }

    #[test]
    fn deleting_visible_paragraph_separator_merges_paragraphs() {
        let document = omw_document::parse(b"first\n\nsecond").unwrap();
        let mut session = EditSession::from_document(&document);
        let projection = VisibleProjection::from_session(&session);
        delete_visible_range(
            &mut session,
            &projection,
            VisualRange::new(VisualPos(5), VisualPos(6)),
        )
        .unwrap();
        assert_eq!(
            VisibleProjection::from_session(&session).text,
            "first\nsecond"
        );
    }

    #[test]
    fn projection_decodes_markdown_backslash_escapes() {
        let document = omw_document::parse(b"\\!hello").unwrap();
        let mut session = EditSession::from_document(&document);
        let projection = VisibleProjection::from_session(&session);
        session
            .insert_text(projection.source_boundaries[0], "!")
            .unwrap();
        assert_eq!(VisibleProjection::from_session(&session).text, "!!hello");
    }

    #[test]
    fn maps_emoji_and_combining_graphemes_without_splitting_surrogates() {
        let text = "a👨‍👩‍👧‍👦e\u{301}";
        assert_eq!(
            utf16_range_to_visual(text, Utf16Range { start: 1, end: 12 }).unwrap(),
            VisualRange::new(VisualPos(1), VisualPos(2))
        );
        assert_eq!(
            utf16_range_to_visual(text, Utf16Range { start: 2, end: 3 }),
            Err(InputError::InvalidUtf16Boundary)
        );
    }
    #[test]
    fn cross_paragraph_selection_replaces_without_touching_markers() {
        let document = omw_document::parse(b"first\n\nsecond").unwrap();
        let mut session = EditSession::from_document(&document);
        commit_text(
            &mut session,
            VisualRange::new(VisualPos(2), VisualPos(9)),
            "X",
        )
        .unwrap();
        assert_eq!(VisibleProjection::from_session(&session).text, "fiXond");
        assert_eq!(String::from_utf8(session.bytes()).unwrap(), "fiXond");
    }
}
