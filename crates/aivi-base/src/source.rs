use std::{
    fmt,
    ops::{Index, Range},
    path::{Path, PathBuf},
    sync::Arc,
};

/// Byte offset into a source file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteIndex(u32);

impl ByteIndex {
    pub const ZERO: Self = Self(0);

    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn as_u32(self) -> u32 {
        self.0
    }

    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl From<u32> for ByteIndex {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

/// Half-open byte span over a source file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    start: ByteIndex,
    end: ByteIndex,
}

impl Span {
    pub fn new(start: ByteIndex, end: ByteIndex) -> Self {
        assert!(start <= end, "span start must not exceed span end");
        Self { start, end }
    }

    pub const fn start(self) -> ByteIndex {
        self.start
    }

    pub const fn end(self) -> ByteIndex {
        self.end
    }

    pub const fn len(self) -> u32 {
        self.end.as_u32() - self.start.as_u32()
    }

    pub const fn is_empty(self) -> bool {
        self.start.as_u32() == self.end.as_u32()
    }

    pub fn contains(self, index: ByteIndex) -> bool {
        self.start <= index && index < self.end
    }

    pub fn join(self, other: Span) -> Span {
        Span::new(
            ByteIndex::new(self.start.as_u32().min(other.start.as_u32())),
            ByteIndex::new(self.end.as_u32().max(other.end.as_u32())),
        )
    }
}

impl From<Range<usize>> for Span {
    fn from(value: Range<usize>) -> Self {
        let start = u32::try_from(value.start).expect("span start exceeded u32::MAX");
        let end = u32::try_from(value.end).expect("span end exceeded u32::MAX");
        Span::new(ByteIndex::new(start), ByteIndex::new(end))
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start.as_u32(), self.end.as_u32())
    }
}

/// Stable file identity used across compiler layers.
///
/// # Safety invariant
///
/// A `FileId` value is only meaningful in the context of the [`SourceDatabase`] (or equivalent
/// source manager) that created it. `FileId` values **must not** be constructed by hand outside
/// of the source manager: doing so is *unsound* because an arbitrary raw integer may not
/// correspond to any registered file, causing out-of-bounds access or silent data corruption
/// when the id is used to index the source file table. Always obtain `FileId` values through
/// [`SourceDatabase::add_file`] or the equivalent source-manager API.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(u32);

impl FileId {
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// File-qualified source span.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SourceSpan {
    file: FileId,
    span: Span,
}

impl SourceSpan {
    pub const fn new(file: FileId, span: Span) -> Self {
        Self { file, span }
    }

    pub const fn file(self) -> FileId {
        self.file
    }

    pub const fn span(self) -> Span {
        self.span
    }

    /// Merge two source spans into a single span covering both.
    ///
    /// Returns `None` if the spans belong to different files. This is intentional;
    /// callers needing cross-file spans should handle `None` explicitly.
    pub fn join(self, other: Self) -> Option<Self> {
        (self.file == other.file).then_some(Self::new(self.file, self.span.join(other.span)))
    }
}

/// Value paired with a source span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spanned<T> {
    pub value: T,
    pub span: SourceSpan,
}

impl<T> Spanned<T> {
    pub fn new(value: T, span: SourceSpan) -> Self {
        Self { value, span }
    }
}

/// One-based line and column used by diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineColumn {
    pub line: usize,
    pub column: usize,
}

/// LSP-compatible position (0-based line/character, UTF-16 code units).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

/// LSP-compatible range (start/end positions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

/// Immutable source file with precomputed line starts.
#[derive(Clone, Debug)]
pub struct SourceFile {
    id: FileId,
    path: PathBuf,
    text: Arc<str>,
    line_starts: Arc<[ByteIndex]>,
}

impl SourceFile {
    pub fn new(id: FileId, path: impl Into<PathBuf>, text: impl Into<Arc<str>>) -> Self {
        let text = text.into();
        let line_starts = compute_line_starts(&text);
        Self {
            id,
            path: path.into(),
            text,
            line_starts,
        }
    }

    pub const fn id(&self) -> FileId {
        self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn full_span(&self) -> SourceSpan {
        SourceSpan::new(self.id, Span::from(0..self.text.len()))
    }

    pub fn span(&self, range: Range<usize>) -> Span {
        assert!(
            range.start <= range.end,
            "range start must not exceed range end"
        );
        assert!(
            range.end <= self.text.len(),
            "range end must stay within the source text"
        );
        debug_assert!(self.text.is_char_boundary(range.start));
        debug_assert!(self.text.is_char_boundary(range.end));
        Span::from(range)
    }

    pub fn source_span(&self, range: Range<usize>) -> SourceSpan {
        SourceSpan::new(self.id, self.span(range))
    }

    pub fn slice(&self, span: Span) -> &str {
        let range = span.start().as_usize()..span.end().as_usize();
        debug_assert!(self.text.is_char_boundary(range.start));
        debug_assert!(self.text.is_char_boundary(range.end));
        &self.text[range]
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    pub fn line_span(&self, zero_based_line: usize) -> Option<Span> {
        let start = *self.line_starts.get(zero_based_line)?;
        let raw_end = self
            .line_starts
            .get(zero_based_line + 1)
            .copied()
            .unwrap_or_else(|| ByteIndex::new(self.text.len() as u32));
        let end = trim_line_end(self.text(), start.as_usize(), raw_end.as_usize());
        Some(Span::from(start.as_usize()..end))
    }

    pub fn line_text(&self, zero_based_line: usize) -> Option<&str> {
        self.line_span(zero_based_line).map(|span| self.slice(span))
    }

    /// Convert a byte offset to an LSP position (0-based line, UTF-16 character offset).
    pub fn offset_to_lsp_position(&self, offset: ByteIndex) -> LspPosition {
        let clamped = offset.as_usize().min(self.text.len());
        let line_index = self
            .line_starts
            .partition_point(|candidate| candidate.as_usize() <= clamped)
            .saturating_sub(1);
        let line_start = self.line_starts[line_index].as_usize();
        let line_slice = &self.text[line_start..clamped];
        LspPosition {
            line: line_index as u32,
            character: utf16_len(line_slice) as u32,
        }
    }

    /// Convert an LSP position (0-based, UTF-16 character offset) to a byte offset.
    ///
    /// Returns `None` if `pos.line` is beyond the last line of the file, or if
    /// `pos.character` exceeds the UTF-16 length of the addressed line.  Callers
    /// that previously relied on silent clamping must handle the `None` case.
    pub fn lsp_position_to_offset(&self, pos: LspPosition) -> Option<ByteIndex> {
        let line_idx = pos.line as usize;
        if line_idx >= self.line_starts.len() {
            return None;
        }
        let line_start = self.line_starts[line_idx].as_usize();
        let line_end = self
            .line_starts
            .get(line_idx + 1)
            .copied()
            .unwrap_or_else(|| ByteIndex::new(self.text.len() as u32))
            .as_usize();
        let line_text = &self.text[line_start..line_end];
        let byte_offset = utf16_to_byte_offset(line_text, pos.character as usize)?;
        Some(ByteIndex::new((line_start + byte_offset) as u32))
    }

    /// Convert a span to an LSP range.
    pub fn span_to_lsp_range(&self, span: Span) -> LspRange {
        LspRange {
            start: self.offset_to_lsp_position(span.start()),
            end: self.offset_to_lsp_position(span.end()),
        }
    }

    pub fn line_column(&self, offset: ByteIndex) -> LineColumn {
        let clamped = offset.as_usize().min(self.text.len());
        let line_index = self
            .line_starts
            .partition_point(|candidate| candidate.as_usize() <= clamped)
            .saturating_sub(1);
        let line_start = self.line_starts[line_index].as_usize();
        LineColumn {
            line: line_index + 1,
            column: clamped - line_start + 1,
        }
    }
}

/// Collection of immutable source files used for span rendering.
#[derive(Clone, Debug, Default)]
/// A collection of source files indexed by their [`FileId`].
///
/// Files may be added with sequentially assigned ids via [`add_file`] or
/// inserted with a pre-assigned id via [`insert`].  The latter supports
/// query-layer databases where file ids may be non-contiguous (e.g. after a
/// file is removed and the id slot is not recycled).
pub struct SourceDatabase {
    files: std::collections::BTreeMap<u32, SourceFile>,
}

impl SourceDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a new source file, assigning the next available sequential id.
    pub fn add_file(&mut self, path: impl Into<PathBuf>, text: impl Into<Arc<str>>) -> FileId {
        let raw = u32::try_from(self.files.len()).expect("source file table exceeded u32::MAX");
        let id = FileId::new(raw);
        self.files.insert(raw, SourceFile::new(id, path, text));
        id
    }

    /// Insert a pre-built [`SourceFile`] using its own [`FileId`] as the key.
    ///
    /// Use this when the caller already manages file ids (e.g. the query-layer
    /// `RootDatabase`) so that ids remain stable even after other files are
    /// removed.
    pub fn insert(&mut self, file: SourceFile) {
        self.files.insert(file.id().as_u32(), file);
    }

    pub fn file(&self, id: FileId) -> Option<&SourceFile> {
        self.files.get(&id.as_u32())
    }

    /// Look up a source file by its [`FileId`] raw value.
    ///
    /// Unlike the sequential-index assumption of an internal `Vec`, this works
    /// correctly when ids are non-contiguous (e.g. after file removal).
    pub fn file_at(&self, id: u32) -> Option<&SourceFile> {
        self.files.get(&id)
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &SourceFile> {
        self.files.values()
    }
}

impl Index<FileId> for SourceDatabase {
    type Output = SourceFile;

    fn index(&self, index: FileId) -> &Self::Output {
        self.file(index).expect("invalid source file id")
    }
}

/// Compute the number of UTF-16 code units in a string slice.
fn utf16_len(s: &str) -> usize {
    s.chars()
        .map(|c| if (c as u32) > 0xFFFF { 2 } else { 1 })
        .sum()
}

/// Convert a UTF-16 code unit offset within a line to a byte offset.
///
/// Returns `None` if `utf16_chars` exceeds the total UTF-16 length of `line`.
fn utf16_to_byte_offset(line: &str, utf16_chars: usize) -> Option<usize> {
    let mut remaining = utf16_chars;
    let mut byte_offset = 0;
    for c in line.chars() {
        if remaining == 0 {
            return Some(byte_offset);
        }
        let width = if (c as u32) > 0xFFFF { 2 } else { 1 };
        if remaining < width {
            // The column lands in the middle of a surrogate pair — out of range.
            return None;
        }
        remaining -= width;
        byte_offset += c.len_utf8();
    }
    // After consuming all characters, remaining must be zero for the position to be valid.
    if remaining == 0 {
        Some(byte_offset)
    } else {
        None
    }
}

fn compute_line_starts(text: &str) -> Arc<[ByteIndex]> {
    let mut starts = Vec::with_capacity(text.bytes().filter(|byte| *byte == b'\n').count() + 1);
    starts.push(ByteIndex::ZERO);
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(ByteIndex::new((index + 1) as u32));
        }
    }
    starts.into()
}

fn trim_line_end(text: &str, start: usize, end: usize) -> usize {
    let bytes = text.as_bytes();
    let mut trimmed = end;
    // Strip a trailing LF first (handles both LF and CRLF line endings).
    if trimmed > start && bytes[trimmed - 1] == b'\n' {
        trimmed -= 1;
    }
    // Strip a trailing CR — covers CRLF (the LF was already removed above)
    // as well as bare CR line endings (old Mac-style \r-only files).
    if trimmed > start && bytes[trimmed - 1] == b'\r' {
        trimmed -= 1;
    }
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_line_columns_and_line_text() {
        let mut database = SourceDatabase::new();
        let file_id = database.add_file("sample.aivi", "value answer = 42\nsignal counter = 0\n");
        let file = &database[file_id];

        assert_eq!(file.line_count(), 3);
        assert_eq!(file.line_text(0), Some("value answer = 42"));
        assert_eq!(file.line_text(1), Some("signal counter = 0"));
        assert_eq!(file.line_text(2), Some(""));
        assert_eq!(
            file.line_column(ByteIndex::new(0)),
            LineColumn { line: 1, column: 1 }
        );

        let counter_offset = file.text().find("counter").unwrap();
        assert_eq!(
            file.line_column(ByteIndex::new(counter_offset as u32)),
            LineColumn { line: 2, column: 8 }
        );
    }

    #[test]
    fn joins_source_spans_on_the_same_file_only() {
        let left = SourceSpan::new(FileId::new(0), Span::from(0..3));
        let right = SourceSpan::new(FileId::new(0), Span::from(4..7));
        let other_file = SourceSpan::new(FileId::new(1), Span::from(4..7));

        assert_eq!(
            left.join(right),
            Some(SourceSpan::new(FileId::new(0), Span::from(0..7)))
        );
        assert_eq!(left.join(other_file), None);
    }
}
