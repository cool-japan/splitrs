use std::path::PathBuf;

/// Per-document in-memory state.
///
/// `line_offsets` stores the byte offset of the start of each line.
/// `line_offsets[0]` is always 0. Used for converting LSP `Position` (which
/// uses UTF-16 code units for `character`) into byte offsets in `text`.
pub struct DocumentState {
    pub text: String,
    pub version: i32,
    /// Byte offset of the start of each line (`line_offsets[0]` is always 0).
    pub line_offsets: Vec<usize>,
}

/// Workspace-level state.
pub struct WorkspaceState {
    pub root: PathBuf,
    pub config_path: Option<PathBuf>,
}

impl DocumentState {
    /// Construct a `DocumentState` from source text and a version counter.
    pub fn new(text: String, version: i32) -> Self {
        let line_offsets = build_line_offsets(&text);
        Self {
            text,
            version,
            line_offsets,
        }
    }
}

/// Build a table of byte offsets for the start of each line.
///
/// `result[0]` is always 0. A newline at byte `i` causes `i + 1` to be pushed.
pub fn build_line_offsets(source: &str) -> Vec<usize> {
    let mut offsets = vec![0usize];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            offsets.push(i + 1);
        }
    }
    offsets
}

/// Convert an LSP `character` value (UTF-16 code units) on a single line to a
/// byte offset relative to the start of that line.
///
/// LSP `Position::character` is in UTF-16 code units, not bytes or `char`
/// counts.  For ASCII-only content the three are identical, but multi-byte
/// characters (e.g. CJK, emoji) differ.
///
/// # Arguments
///
/// * `line` — the content of the line (NOT including the trailing newline)
/// * `utf16_col` — the LSP `Position::character` value
///
/// # Returns
///
/// The byte offset from the start of `line` that corresponds to `utf16_col`.
/// If `utf16_col` exceeds the line length, the returned offset is clamped to
/// `line.len()`.
pub fn utf16_char_to_byte_offset(line: &str, utf16_col: u32) -> usize {
    let mut utf16_count = 0u32;
    let mut byte_offset = 0usize;
    for ch in line.chars() {
        if utf16_count >= utf16_col {
            break;
        }
        utf16_count += ch.len_utf16() as u32;
        byte_offset += ch.len_utf8();
    }
    byte_offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_line_offsets_ascii() {
        let source = "fn foo() {}\nfn bar() {}\n";
        let offsets = build_line_offsets(source);
        assert_eq!(offsets[0], 0);
        assert_eq!(offsets[1], 12); // "fn foo() {}\n" is 12 bytes
        assert_eq!(offsets[2], 24);
    }

    #[test]
    fn test_build_line_offsets_empty() {
        let offsets = build_line_offsets("");
        assert_eq!(offsets, vec![0]);
    }

    #[test]
    fn test_utf16_roundtrip_ascii() {
        let line = "let x = 42;";
        for col in 0..=(line.len() as u32) {
            let byte = utf16_char_to_byte_offset(line, col);
            assert_eq!(byte, col as usize, "col={col}");
        }
    }

    #[test]
    fn test_utf16_roundtrip_multibyte() {
        // '€' is U+20AC: 3 UTF-8 bytes, 1 UTF-16 code unit
        // '𝄞' is U+1D11E (musical symbol G clef): 4 UTF-8 bytes, 2 UTF-16 code units
        let line = "a€b𝄞c";
        // UTF-16 positions: a=0, €=1, b=2, 𝄞=3 (takes 2 units: 3 & 4), c=5
        assert_eq!(utf16_char_to_byte_offset(line, 0), 0); // 'a' start
        assert_eq!(utf16_char_to_byte_offset(line, 1), 1); // '€' start (1 byte past 'a')
        assert_eq!(utf16_char_to_byte_offset(line, 2), 4); // 'b' start (1 + 3 bytes for '€')
        assert_eq!(utf16_char_to_byte_offset(line, 3), 5); // '𝄞' start
        assert_eq!(utf16_char_to_byte_offset(line, 5), 9); // 'c' start (5 + 4 bytes for '𝄞')
    }

    #[test]
    fn test_document_state_construction() {
        let text = "line one\nline two\n".to_string();
        let doc = DocumentState::new(text.clone(), 1);
        assert_eq!(doc.version, 1);
        assert_eq!(doc.text, text);
        assert_eq!(doc.line_offsets[0], 0);
        assert_eq!(doc.line_offsets[1], 9); // "line one\n" is 9 bytes
    }
}
