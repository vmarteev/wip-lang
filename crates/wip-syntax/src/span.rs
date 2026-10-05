use std::fmt;
use std::ops::Range;

/// A half-open byte range `lo..hi` into the source file.
///
/// A span does not name its file: what holds it says which file it is in.
/// Offsets are `u32`; the driver rejects files larger than 4 GiB.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub lo: u32,
    pub hi: u32,
}

impl Span {
    pub fn new(lo: u32, hi: u32) -> Span {
        debug_assert!(lo <= hi, "inverted span {lo}..{hi}");
        Span { lo, hi }
    }

    /// An empty span at `pos`, for "expected X here" diagnostics and insertions.
    pub fn at(pos: u32) -> Span {
        Span { lo: pos, hi: pos }
    }

    /// The smallest span covering both `self` and `other`.
    pub fn to(self, other: Span) -> Span {
        Span::new(self.lo.min(other.lo), self.hi.max(other.hi))
    }

    pub fn len(self) -> u32 {
        self.hi - self.lo
    }

    pub fn is_empty(self) -> bool {
        self.lo == self.hi
    }

    pub fn range(self) -> Range<usize> {
        self.lo as usize..self.hi as usize
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.lo, self.hi)
    }
}

/// Maps byte offsets to 1-based line and column numbers.
///
/// Columns count Unicode scalar values, not bytes, which is also how `ariadne`
/// numbers them — so `wip lex` and rendered diagnostics agree.
pub struct LineIndex {
    line_starts: Vec<u32>,
}

impl LineIndex {
    pub fn new(src: &str) -> LineIndex {
        let newlines = src.bytes().enumerate().filter(|&(_, b)| b == b'\n');
        let line_starts = std::iter::once(0)
            .chain(newlines.map(|(i, _)| i as u32 + 1))
            .collect();
        LineIndex { line_starts }
    }

    pub fn line_col(&self, src: &str, pos: u32) -> (u32, u32) {
        // Line 0 starts at offset 0, so the partition point is at least 1.
        let line = self.line_starts.partition_point(|&start| start <= pos) - 1;
        let start = self.line_starts[line] as usize;
        let col = src[start..pos as usize].chars().count();
        (line as u32 + 1, col as u32 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_count_chars_not_bytes() {
        let src = "ab\n\"é\" x\n";
        let index = LineIndex::new(src);
        assert_eq!(index.line_col(src, 0), (1, 1));
        assert_eq!(index.line_col(src, 2), (1, 3));
        assert_eq!(index.line_col(src, 3), (2, 1));
        // `é` is two bytes, so `x` is at byte 8 but column 5.
        assert_eq!(index.line_col(src, 8), (2, 5));
        assert_eq!(index.line_col(src, src.len() as u32), (3, 1));
    }
}
