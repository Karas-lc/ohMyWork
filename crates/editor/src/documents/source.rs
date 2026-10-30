use crate::{ParseError, TaskCommitError, serialize::TextPatch};

pub type SourceRope = gpui_kit::component::input::Rope;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceSelection {
    pub anchor: usize,
    pub head: usize,
}

#[derive(Clone)]
struct HistoryEdge {
    before: SourceSelection,
    after: SourceSelection,
}

#[derive(Clone)]
pub(super) struct SourceSession {
    states: Vec<SourceRope>,
    edges: Vec<HistoryEdge>,
    cursor: usize,
}

impl SourceSession {
    pub(super) fn new(source: SourceRope) -> Self {
        Self { states: vec![source], edges: Vec::new(), cursor: 0 }
    }

    pub(super) fn source(&self) -> &SourceRope { &self.states[self.cursor] }
    pub(super) fn replace(&mut self, source: SourceRope, before: SourceSelection, after: SourceSelection) {
        self.states.truncate(self.cursor + 1);
        self.edges.truncate(self.cursor);
        self.states.push(source);
        self.edges.push(HistoryEdge { before, after });
        self.cursor += 1;
    }
    pub(super) fn undo(&mut self) -> Option<SourceSelection> {
        if self.cursor == 0 { return None; }
        let selection = self.edges[self.cursor - 1].before;
        self.cursor -= 1;
        Some(selection)
    }
    pub(super) fn redo(&mut self) -> Option<SourceSelection> {
        let edge = self.edges.get(self.cursor)?;
        let selection = edge.after;
        self.cursor += 1;
        Some(selection)
    }
    pub(super) fn reset(&mut self, source: SourceRope) {
        *self = Self::new(source);
    }

    pub(super) fn with_committed_task_patch(
        &self,
        baseline: &[u8],
        task_range: std::ops::Range<usize>,
        patch: &TextPatch,
    ) -> Result<Self, TaskCommitError> {
        let start = patch.range.start.0 as usize;
        let end = patch.range.end.0 as usize;
        if start > end || end > baseline.len() || !is_utf8_boundary(baseline, start) || !is_utf8_boundary(baseline, end)
            || task_range.start > task_range.end || task_range.end > baseline.len()
            || start < task_range.start || end > task_range.end
        {
            return Err(TaskCommitError::InvalidFieldSpan);
        }
        let original = &baseline[start..end];
        if crate::fnv1a64(original) != patch.expected_hash {
            return Err(TaskCommitError::PatchFailed("expected source bytes do not match".into()));
        }
        let baseline_prefix = &baseline[..task_range.end];
        let prefix_hash = blake3::hash(baseline_prefix);
        let mut bytes_by_state = Vec::with_capacity(self.states.len());
        for state in &self.states {
            let bytes = rope_bytes(state);
            if bytes.len() < task_range.end
                || &bytes[..task_range.end] != baseline_prefix
                || blake3::hash(&bytes[..task_range.end]) != prefix_hash
                || bytes.get(start..end) != Some(original)
            {
                return Err(TaskCommitError::CannotLocateUnique {
                    reason: "任务源范围或撤销历史已变化；请先处理正文编辑后重试".into(),
                });
            }
            bytes_by_state.push(bytes);
        }
        let mut states = Vec::with_capacity(bytes_by_state.len());
        for mut bytes in bytes_by_state {
            bytes.splice(start..end, patch.replacement.as_bytes().iter().copied());
            if bytes.len() > u32::MAX as usize { return Err(TaskCommitError::InvalidFieldSpan); }
            let text = std::str::from_utf8(&bytes).map_err(|_| TaskCommitError::InvalidFieldSpan)?;
            states.push(SourceRope::from_str(text));
        }
        let mut rebased = self.clone();
        rebased.states = states;
        let delta = patch.replacement.len() as isize - (end - start) as isize;
        for edge in &mut rebased.edges {
            edge.before.anchor = transform_offset(edge.before.anchor, start, end, patch.replacement.len(), delta);
            edge.before.head = transform_offset(edge.before.head, start, end, patch.replacement.len(), delta);
            edge.after.anchor = transform_offset(edge.after.anchor, start, end, patch.replacement.len(), delta);
            edge.after.head = transform_offset(edge.after.head, start, end, patch.replacement.len(), delta);
        }
        Ok(rebased)
    }
}

pub fn decode_source(bytes: &[u8]) -> Result<SourceRope, ParseError> {
    crate::parse::detect_encoding(bytes)?;
    let source = std::str::from_utf8(bytes).map_err(|error| ParseError::InvalidUtf8 { position: error.valid_up_to() })?;
    if bytes.len() > u32::MAX as usize {
        return Err(ParseError::DocumentTooLarge { size: bytes.len() });
    }
    Ok(SourceRope::from_str(source))
}

pub(super) fn valid_selection(rope: &SourceRope, selection: SourceSelection) -> bool {
    valid_offset(rope, selection.anchor) && valid_offset(rope, selection.head)
}

fn valid_offset(rope: &SourceRope, byte: usize) -> bool {
    if byte > rope.len() { return false; }
    rope.char_to_byte_idx(rope.byte_to_char_idx(byte)) == byte
}

fn is_utf8_boundary(bytes: &[u8], index: usize) -> bool {
    index <= bytes.len() && (index == bytes.len() || bytes[index] & 0b1100_0000 != 0b1000_0000)
}

fn rope_bytes(rope: &SourceRope) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(rope.len());
    for chunk in rope.chunks() { bytes.extend_from_slice(chunk.as_bytes()); }
    bytes
}

fn transform_offset(position: usize, start: usize, end: usize, replacement_len: usize, delta: isize) -> usize {
    if start == end {
        if position < start { position } else { position + replacement_len }
    } else if position < start {
        position
    } else if position >= end {
        position.saturating_add_signed(delta)
    } else {
        start + replacement_len
    }
}
