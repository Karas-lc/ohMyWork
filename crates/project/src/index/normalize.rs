use std::ops::Range;

use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

/// A normalized string plus the source byte range represented by each output scalar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedText {
    pub text: String,
    source_ranges: Vec<Range<usize>>,
}

impl NormalizedText {
    pub fn source_range(&self, normalized_range: Range<usize>) -> Option<Range<usize>> {
        if normalized_range.start >= normalized_range.end
            || normalized_range.end > self.source_ranges.len()
        {
            return None;
        }
        let start = self.source_ranges[normalized_range.start].start;
        let end = self.source_ranges[normalized_range.end - 1].end;
        Some(start..end)
    }
}

/// Apply NFKC followed by Unicode scalar lowercase while retaining source mapping.
pub fn normalize(source: &str) -> NormalizedText {
    let mut text = String::new();
    let mut source_ranges = Vec::new();

    for (start, grapheme) in source.grapheme_indices(true) {
        let end = start + grapheme.len();
        for scalar in grapheme.nfkc().flat_map(char::to_lowercase) {
            text.push(scalar);
            source_ranges.push(start..end);
        }
    }

    NormalizedText {
        text,
        source_ranges,
    }
}

pub fn contains(source: &str, normalized_query: &str) -> bool {
    !normalized_query.is_empty() && normalize(source).text.contains(normalized_query)
}

/// Return a source-faithful context snippet, or `None` when no mapped match exists.
pub fn snippet(source: &str, query: &str, context: usize) -> Option<String> {
    let normalized_source = normalize(source);
    let normalized_query = normalize(query).text;
    let start = normalized_source.text.find(&normalized_query)?;
    let normalized_range = start..start + normalized_query.len();
    let source_range =
        normalized_source.source_range(scalar_range(&normalized_source.text, normalized_range))?;

    let boundaries = source
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(source.len()))
        .collect::<Vec<_>>();
    let source_start = boundaries
        .partition_point(|offset| *offset < source_range.start)
        .min(boundaries.len() - 1);
    let source_end = boundaries
        .partition_point(|offset| *offset < source_range.end)
        .min(boundaries.len() - 1);
    let start = source_start.saturating_sub(context);
    let end = (source_end + context).min(boundaries.len() - 1);
    let mut result = source[boundaries[start]..boundaries[end]].to_owned();
    if start > 0 {
        result.insert_str(0, "…");
    }
    if end < boundaries.len() - 1 {
        result.push('…');
    }
    Some(result)
}

fn scalar_range(text: &str, byte_range: Range<usize>) -> Range<usize> {
    let start = text[..byte_range.start].chars().count();
    let end = start + text[byte_range].chars().count();
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_width_case_and_combining_unicode_with_source_mapping() {
        let normalized = normalize("ＡＢＣ e\u{301} 😀");
        assert_eq!(normalized.text, "abc é 😀");
        let range = normalized
            .text
            .find('é')
            .map(|offset| scalar_range(&normalized.text, offset..offset + 'é'.len_utf8()))
            .and_then(|range| normalized.source_range(range))
            .expect("mapped accent");
        assert_eq!(&"ＡＢＣ e\u{301} 😀"[range], "e\u{301}");
    }

    #[test]
    fn zero_context_keeps_only_the_mapped_source_scalars() {
        let fragment = snippet("前ＡＢＣ后", "abc", 0).expect("normalized query matches");
        assert_eq!(
            fragment.trim_matches('…').chars().collect::<Vec<_>>(),
            ['Ａ', 'Ｂ', 'Ｃ']
        );
    }
}
