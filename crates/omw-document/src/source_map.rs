//! 源文本的字节行映射。
//!
//! `line_start_bytes` 记录每一行的起始字节偏移（第 1 行的偏移恒为 0），
//! 用于把 1-based 的 `(line, column)`（column 按 UTF-8 字节计数，与
//! Comrak 默认 `sourcepos` 语义一致）换算为字节偏移。支持三类行结束：
//! `\n`、`\r\n`、孤立 `\r`。

use crate::model::ByteOffset;

/// 行起始字节偏移映射。
///
/// `line_starts()[i]` 是第 `i + 1` 行的起始字节偏移；`line_starts()` 长度即
/// Comrak 意义上的"行数 + 1"（末行后可能存在一个虚拟空行，用于 EOF 换算）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceMap {
    line_start_bytes: Vec<ByteOffset>,
}

impl SourceMap {
    /// 由原始字节建立行映射，不要求文本以行结束符收尾。
    pub fn new(text: &[u8]) -> SourceMap {
        let mut starts = Vec::with_capacity(text.len() / 40 + 2);
        starts.push(ByteOffset(0));
        let mut i = 0usize;
        while i < text.len() {
            match text[i] {
                b'\n' => {
                    starts.push(ByteOffset(u32::try_from(i + 1).unwrap_or(u32::MAX)));
                    i += 1;
                }
                b'\r' => {
                    if text.get(i + 1) == Some(&b'\n') {
                        starts.push(ByteOffset(u32::try_from(i + 2).unwrap_or(u32::MAX)));
                        i += 2;
                    } else {
                        starts.push(ByteOffset(u32::try_from(i + 1).unwrap_or(u32::MAX)));
                        i += 1;
                    }
                }
                _ => i += 1,
            }
        }
        SourceMap {
            line_start_bytes: starts,
        }
    }

    /// 全部行起始偏移（第 `i` 项对应第 `i + 1` 行）。
    pub fn line_starts(&self) -> &[ByteOffset] {
        &self.line_start_bytes
    }

    /// 有效行数（最后可能含一个虚拟空行）。
    pub fn line_count(&self) -> usize {
        self.line_start_bytes.len()
    }

    /// 把 1-based `(line, column)`（column 为字节计数，可为 0，表示落在行首
    /// 之前的空偏移，主要用于行尾换算）换算为字节偏移。越界或非法时返回
    /// `None`；调用方必须据此把节点标记为不可归属，而不是 panic。
    pub fn offset_of(
        &self,
        line: usize,
        column: usize,
        text_len: ByteOffset,
    ) -> Option<ByteOffset> {
        if line == 0 {
            return None;
        }
        let start = *self.line_start_bytes.get(line - 1)?;
        // column == 0 只允许作为行首偏移（结束位置换算用）。
        if column == 0 {
            return (start <= text_len).then_some(start);
        }
        let offset = start.u64_checked_add(column as u64 - 1)?;
        let offset = ByteOffset::try_from_u64(offset)?;
        (offset <= text_len).then_some(offset)
    }

    /// 把字节偏移换算为 1-based `(line, column)`（诊断用）。
    pub fn line_col_of(&self, offset: ByteOffset) -> (usize, usize) {
        match self
            .line_start_bytes
            .binary_search_by(|s| s.0.cmp(&offset.0))
        {
            Ok(idx) => (idx + 1, 1),
            Err(idx) => {
                let line = idx.max(1);
                let column =
                    usize::try_from(offset.0.saturating_sub(self.line_start_bytes[line - 1].0))
                        .unwrap_or(usize::MAX)
                        + 1;
                (line, column)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ByteRange;

    fn map_of(s: &str) -> SourceMap {
        SourceMap::new(s.as_bytes())
    }

    #[test]
    fn line_starts_lf_crlf_lone_cr() {
        let m = map_of("a\nb\r\nc\rd");
        assert_eq!(
            m.line_starts(),
            &[ByteOffset(0), ByteOffset(2), ByteOffset(5), ByteOffset(7)]
        );
    }

    #[test]
    fn trailing_newline_has_virtual_line() {
        assert_eq!(map_of("hi\n").line_count(), 2);
        assert_eq!(map_of("hi").line_count(), 1);
        assert_eq!(map_of("").line_count(), 1);
    }

    #[test]
    fn offset_of_and_back() {
        let text = "中\n好😀\n";
        let m = map_of(text);
        let len = ByteOffset(u32::try_from(text.len()).unwrap());
        // 中=3 字节、\n、好=3 字节（行 2 从字节 4 起）、😀=4 字节、尾 \n。
        assert_eq!(m.offset_of(1, 1, len), Some(ByteOffset(0)));
        assert_eq!(m.offset_of(1, 3, len), Some(ByteOffset(2))); // 中的末字节
        assert_eq!(m.offset_of(2, 5, len), Some(ByteOffset(8))); // 行2: 4+(5-1) → 😀 首字节
        assert_eq!(m.offset_of(2, 0, len), Some(ByteOffset(4)));
        assert_eq!(m.offset_of(3, 0, len), Some(ByteOffset(12))); // 尾 \n 后的虚拟行
        assert_eq!(m.offset_of(99, 1, len), None);
        assert_eq!(m.offset_of(0, 1, len), None);
        assert_eq!(m.offset_of(3, 2, len), None);
    }

    #[test]
    fn ranges_use_inclusive_end_column() {
        // "ab\ncd"：行 1 从 0 起，行 2 从 3 起。
        // 转换规则：end = line_start + end_col（闭端列直接是半开界）。
        let text = "ab\ncd";
        let m = map_of(text);
        let len = ByteOffset(5);
        // offset_of(line, col) 返回该列字符的起始字节；闭端转换需 +1 越过末字符。
        let start = m.offset_of(1, 1, len).unwrap(); // 0 = 'a'
        assert_eq!(start, ByteOffset(0));
        let end = m.offset_of(1, 2, len).unwrap(); // 1 = 'b'
        assert_eq!(
            ByteRange::new(
                start,
                ByteOffset(u32::try_from(end.to_usize() + 1).unwrap())
            )
            .to_byte_range(),
            0..2
        );
        assert_eq!(m.offset_of(2, 2, len), Some(ByteOffset(4))); // 'd' 起始
        assert_eq!(m.line_col_of(ByteOffset(0)), (1, 1));
        assert_eq!(m.line_col_of(ByteOffset(4)), (2, 2));
    }
}
