/// Half-open byte offset span `[start, end)` within a UTF-8 source string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Span {
    /// Inclusive starting byte offset.
    pub start: u32,
    /// Exclusive ending byte offset.
    pub end: u32,
}

impl Span {
    /// Creates a new `Span` from byte offsets `[start, end)`.
    #[inline]
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// Creates a zero-length span at `pos`.
    #[inline]
    pub const fn empty(pos: u32) -> Self {
        Self {
            start: pos,
            end: pos,
        }
    }

    /// Merges two spans into the smallest span covering both.
    #[inline]
    pub fn join(self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}
