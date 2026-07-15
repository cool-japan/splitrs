//! Verbatim source slicing: map `proc_macro2::Span` (line/column) back to exact
//! original source byte ranges so split items can be emitted byte-for-byte,
//! preserving inline `//` comments and original formatting that `prettyplease`
//! would otherwise strip or reflow.

// `span_slice` / `item_verbatim` are part of this module's public API and are
// exercised by the unit tests, but the binary target currently only calls
// `item_verbatim_with_indent` / `impl_header_verbatim`. The binary recompiles
// these modules directly (separate from the lib target), so the unused public
// methods would emit `dead_code` on the bin build. Mirror the `file_analyzer`
// pattern and allow it: these items are an intentional shared internal API.
#![allow(dead_code)]

use proc_macro2::{LineColumn, Span};

/// Index over the original source enabling (line, column) -> byte-offset mapping
/// and exact slice extraction for spans.
pub struct SourceMap<'a> {
    source: &'a str,
    /// Byte offset at which each line starts. `line_starts[0]` == 0 (line 1).
    line_starts: Vec<usize>,
}

impl<'a> SourceMap<'a> {
    /// Build a line-offset index over `source`. Line 1 starts at byte 0; each
    /// subsequent entry is the byte offset immediately AFTER a `\n`.
    pub fn new(source: &'a str) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            source,
            line_starts,
        }
    }

    /// Convert a 1-based line / 0-based UTF-8 *char* column to a byte offset.
    /// Walks `column` chars from the line start so multibyte chars are handled.
    /// Returns `None` if the line is out of range. Clamps a column past
    /// end-of-line to the line's end (defensive; proc-macro2 columns are valid).
    fn line_col_to_byte(&self, line: usize, column: usize) -> Option<usize> {
        if line == 0 || line > self.line_starts.len() {
            return None;
        }
        let line_start = self.line_starts[line - 1];
        // Determine this line's exclusive end (start of next line, or source end).
        let line_end = self
            .line_starts
            .get(line) // next line's start
            .copied()
            .unwrap_or(self.source.len());
        let line_slice = &self.source[line_start..line_end];
        // Walk `column` chars.
        let mut byte = line_start;
        let mut chars = line_slice.char_indices();
        for _ in 0..column {
            match chars.next() {
                Some((_, c)) => byte += c.len_utf8(),
                None => return Some(line_end), // clamp
            }
        }
        Some(byte)
    }

    /// Exact source slice for `span` (start..end), or `None` if either endpoint
    /// maps out of range. Uses `Span::start()`/`Span::end()` (real `LineColumn`
    /// when `span-locations` is enabled).
    pub fn span_slice(&self, span: Span) -> Option<&'a str> {
        let start = span.start();
        let end = span.end();
        let start_byte = self.line_col_to_byte(start.line, start.column)?;
        let end_byte = self.line_col_to_byte(end.line, end.column)?;
        if start_byte > end_byte || end_byte > self.source.len() {
            return None;
        }
        Some(&self.source[start_byte..end_byte])
    }

    /// Earliest start `LineColumn` among an item's own span start and all its
    /// attributes' span starts. Captures leading `#[...]` and `///` doc comments
    /// (attributes ARE part of the item and carry spans), which begin BEFORE the
    /// item's main token span.
    fn earliest_start(item_span: Span, attrs: &[syn::Attribute]) -> LineColumn {
        use syn::spanned::Spanned;
        let mut earliest = item_span.start();
        for attr in attrs {
            let s = attr.span().start();
            if (s.line, s.column) < (earliest.line, earliest.column) {
                earliest = s;
            }
        }
        earliest
    }

    /// Verbatim slice for an item INCLUDING its leading attributes/doc comments,
    /// from the earliest attribute/item start through the item span end (closing
    /// brace). Inner `//` comments fall naturally within the braces.
    ///
    /// NOTE: starts at the first non-whitespace token column, so the FIRST line's
    /// leading indentation is not included. Use [`Self::item_verbatim_with_indent`]
    /// when you need the original indentation preserved (e.g. emitting methods
    /// inside a re-synthesised `impl` block).
    pub fn item_verbatim(&self, item_span: Span, attrs: &[syn::Attribute]) -> Option<&'a str> {
        let start = Self::earliest_start(item_span, attrs);
        let end = item_span.end();
        let start_byte = self.line_col_to_byte(start.line, start.column)?;
        let end_byte = self.line_col_to_byte(end.line, end.column)?;
        if start_byte > end_byte || end_byte > self.source.len() {
            return None;
        }
        Some(&self.source[start_byte..end_byte])
    }

    /// Like [`Self::item_verbatim`] but starts at the BEGINNING of the line the
    /// earliest token sits on, preserving that line's original leading
    /// indentation. Subsequent lines already retain their indentation (they are
    /// mid-slice). Used for methods/trait-impls so they keep original layout.
    pub fn item_verbatim_with_indent(
        &self,
        item_span: Span,
        attrs: &[syn::Attribute],
    ) -> Option<&'a str> {
        let start = Self::earliest_start(item_span, attrs);
        if start.line == 0 || start.line > self.line_starts.len() {
            return None;
        }
        let start_byte = self.line_starts[start.line - 1]; // include leading indentation
        let end = item_span.end();
        let end_byte = self.line_col_to_byte(end.line, end.column)?;
        if start_byte > end_byte || end_byte > self.source.len() {
            return None;
        }
        Some(&self.source[start_byte..end_byte])
    }

    /// Verbatim text of an impl block's header: from the impl's earliest start
    /// (incl. attributes) up to AND INCLUDING the opening `{`. Lets callers wrap
    /// verbatim method bodies in the byte-faithful original `impl ... {` line.
    pub fn impl_header_verbatim(&self, item: &syn::ItemImpl) -> Option<String> {
        use syn::spanned::Spanned;
        let start = Self::earliest_start(item.span(), &item.attrs);
        let start_byte = self.line_col_to_byte(start.line, start.column)?;
        // Opening-brace location via the brace token's delimiter span.
        let brace_open = item.brace_token.span.open();
        let bo = brace_open.start();
        let brace_open_byte = self.line_col_to_byte(bo.line, bo.column)?;
        // `brace_open_byte` points AT the `{`; include it (ASCII `{` is 1 byte).
        let end = brace_open_byte + 1;
        if start_byte > end || end > self.source.len() {
            return None;
        }
        Some(self.source[start_byte..end].to_string())
    }

    /// Byte range `[start, end)` covering an item's verbatim slice INCLUDING its
    /// leading attributes/doc comments, starting at the BEGINNING of the line
    /// the earliest token sits on (so indentation is retained) and ending at the
    /// item's span end (closing brace) extended forward through the remaining
    /// whitespace of that physical line and its trailing `\n`.
    ///
    /// Unlike [`Self::item_verbatim_with_indent`] (which returns the text), this
    /// returns byte offsets so a caller can *cut* the item out of the original
    /// source — e.g. relocating a `#[cfg(test)] mod tests { … }` block out of a
    /// regenerated `mod.rs` while leaving every surrounding production comment
    /// byte-for-byte intact. Returns `None` if either endpoint maps out of range.
    pub fn item_cut_range(
        &self,
        item_span: Span,
        attrs: &[syn::Attribute],
    ) -> Option<(usize, usize)> {
        let start = Self::earliest_start(item_span, attrs);
        if start.line == 0 || start.line > self.line_starts.len() {
            return None;
        }
        let start_byte = self.line_starts[start.line - 1];
        let end = item_span.end();
        let mut end_byte = self.line_col_to_byte(end.line, end.column)?;
        // Extend through trailing whitespace to AND INCLUDING the next newline so
        // the cut leaves no dangling blank fragment. Stop at the first non-ws
        // byte (defensive: something else shares the closing brace's line).
        let bytes = self.source.as_bytes();
        while end_byte < bytes.len() {
            match bytes[end_byte] {
                b'\n' => {
                    end_byte += 1;
                    break;
                }
                b' ' | b'\t' | b'\r' => end_byte += 1,
                _ => break,
            }
        }
        if start_byte > end_byte || end_byte > self.source.len() {
            return None;
        }
        Some((start_byte, end_byte))
    }

    /// The leading whitespace (indentation) of the line the item's earliest
    /// token sits on. Empty for a top-level (column-0) item. Used to re-indent a
    /// replacement (`mod NAME;`) spliced in where the item was cut.
    pub fn line_leading_indent(
        &self,
        item_span: Span,
        attrs: &[syn::Attribute],
    ) -> Option<&'a str> {
        let start = Self::earliest_start(item_span, attrs);
        if start.line == 0 || start.line > self.line_starts.len() {
            return None;
        }
        let line_start = self.line_starts[start.line - 1];
        let tok_byte = self.line_col_to_byte(start.line, start.column)?;
        if line_start > tok_byte || tok_byte > self.source.len() {
            return None;
        }
        Some(&self.source[line_start..tok_byte])
    }
}

// ── Comment-loss audit (safety net) ─────────────────────────────────────────
//
// SplitRS relocates code rather than deleting it, so every comment in an input
// file must resurface in *some* generated file. When regeneration goes through
// `prettyplease`/`quote` (an AST round-trip) instead of a verbatim byte slice,
// every non-doc `//`/`/* */` comment is silently dropped — the AST simply does
// not carry them. These helpers let a caller compare input vs. combined output
// and warn the user (never silently destroy design rationale). Doc comments
// (`///`, `//!`, `/** */`, `/*! */`) are intentionally excluded: they survive
// the round-trip as `#[doc]` attributes and so cannot be reliably matched as
// literal comment text.

/// A scanned comment plus whether it is a doc comment.
struct ScannedComment {
    text: String,
    is_doc: bool,
}

/// UTF-8 byte length implied by a leading byte (1 for ASCII / continuation).
fn utf8_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first >> 5 == 0b110 {
        2
    } else if first >> 4 == 0b1110 {
        3
    } else if first >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

/// `true` when `b[i]` begins a raw-string opener `r"` / `r#"` / `r##"` …
fn is_raw_string_start(b: &[u8], i: usize) -> bool {
    if b.get(i) != Some(&b'r') {
        return false;
    }
    let mut j = i + 1;
    while b.get(j) == Some(&b'#') {
        j += 1;
    }
    b.get(j) == Some(&b'"')
}

/// Index just past a raw string that starts at `i` (on the `r`).
fn skip_raw_string(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i + 1;
    let mut hashes = 0usize;
    while b.get(j) == Some(&b'#') {
        hashes += 1;
        j += 1;
    }
    j += 1; // past the opening `"`
    while j < n {
        if b[j] == b'"' {
            let mut k = j + 1;
            let mut cnt = 0usize;
            while cnt < hashes && b.get(k) == Some(&b'#') {
                cnt += 1;
                k += 1;
            }
            if cnt == hashes {
                return k;
            }
            j += 1;
        } else {
            j += 1;
        }
    }
    n
}

/// Index just past a normal string that starts at `i` (on the opening `"`).
fn skip_string(b: &[u8], i: usize) -> usize {
    let n = b.len();
    let mut j = i + 1;
    while j < n {
        match b[j] {
            b'\\' => j += 2,
            b'"' => return j + 1,
            _ => j += 1,
        }
    }
    n
}

/// Index just past a char literal starting at `i` (on `'`), OR `i + 1` when the
/// `'` is actually a lifetime/label tick (so the scanner does not swallow the
/// rest of the line as a "char literal", the classic `&'a str` trap).
fn skip_char_or_lifetime(b: &[u8], i: usize) -> usize {
    let n = b.len();
    if i + 1 >= n {
        return i + 1;
    }
    if b[i + 1] == b'\\' {
        // Escape char literal: '\n', '\'', '\\', '\u{7f}', …
        let mut j = i + 2;
        while j < n && b[j] != b'\'' {
            if b[j] == b'\\' {
                j += 2;
            } else {
                j += 1;
            }
        }
        return (j + 1).min(n);
    }
    let ch_len = utf8_len(b[i + 1]);
    if b.get(i + 1 + ch_len) == Some(&b'\'') {
        return i + 1 + ch_len + 1;
    }
    // Lifetime / loop label (`'a`, `'static`) — not a char literal.
    i + 1
}

/// Walk `src` byte-by-byte, tracking string/char/raw-literal state so a `//` or
/// `/*` inside a literal is not mistaken for a comment, and return every comment
/// found (trimmed). Not a full lexer — it audits comment survival, so a rare
/// misclassification only skews a diagnostic, never the emitted code.
fn scan_comments(src: &str) -> Vec<ScannedComment> {
    let b = src.as_bytes();
    let n = b.len();
    let mut i = 0usize;
    let mut out = Vec::new();
    while i < n {
        if b[i] == b'/' && i + 1 < n && b[i + 1] == b'/' {
            // Line comment. Doc: `///` (but not `////`) or `//!`.
            let third = b.get(i + 2).copied();
            let is_doc =
                (third == Some(b'/') && b.get(i + 3) != Some(&b'/')) || third == Some(b'!');
            let start = i;
            i += 2;
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            out.push(ScannedComment {
                text: src[start..i].trim().to_string(),
                is_doc,
            });
        } else if b[i] == b'/' && i + 1 < n && b[i + 1] == b'*' {
            // Block comment (nesting). Doc: `/**` (but not `/**/`) or `/*!`.
            let third = b.get(i + 2).copied();
            let is_doc =
                (third == Some(b'*') && b.get(i + 3) != Some(&b'/')) || third == Some(b'!');
            let start = i;
            i += 2;
            let mut depth = 1usize;
            while i < n && depth > 0 {
                if i + 1 < n && b[i] == b'/' && b[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if i + 1 < n && b[i] == b'*' && b[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            out.push(ScannedComment {
                text: src[start..i].trim().to_string(),
                is_doc,
            });
        } else if b[i] == b'b' && is_raw_string_start(b, i + 1) {
            i = skip_raw_string(b, i + 1); // byte raw string `br#"…"#`
        } else if is_raw_string_start(b, i) {
            i = skip_raw_string(b, i);
        } else if b[i] == b'"' {
            i = skip_string(b, i);
        } else if b[i] == b'\'' {
            i = skip_char_or_lifetime(b, i);
        } else {
            i += 1;
        }
    }
    out
}

/// The trimmed text of every *non-doc* comment in `src`, in source order,
/// duplicates retained. See the module-level audit note above.
pub fn extract_comment_texts(src: &str) -> Vec<String> {
    scan_comments(src)
        .into_iter()
        .filter(|c| !c.is_doc && !c.text.is_empty())
        .map(|c| c.text)
        .collect()
}

/// Non-doc comment texts present in `input` but missing from `output` (multiset
/// difference). A non-empty result means regeneration silently dropped those
/// comments — the caller should warn rather than destroy them quietly.
pub fn dropped_comment_texts(input: &str, output: &str) -> Vec<String> {
    let mut have: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for t in extract_comment_texts(output) {
        *have.entry(t).or_insert(0) += 1;
    }
    let mut dropped = Vec::new();
    for t in extract_comment_texts(input) {
        match have.get_mut(&t) {
            Some(count) if *count > 0 => *count -= 1,
            _ => dropped.push(t),
        }
    }
    dropped
}

/// Warn (never fail) on stderr when regenerating `output` from `input` dropped
/// any non-doc comment. SplitRS relocates code rather than deleting it, so a
/// dropped comment is always design rationale being destroyed — surface it
/// instead of losing it silently. Doc comments are excluded (they survive as
/// `#[doc]` attributes). Only the first line of each dropped comment is shown so
/// a multi-line block stays readable, and the list is capped.
pub fn warn_if_comments_dropped(input: &str, output: &str) {
    let dropped = dropped_comment_texts(input, output);
    if dropped.is_empty() {
        return;
    }
    eprintln!(
        "warning: SplitRS dropped {} inline comment(s) while regenerating output \
         (non-doc `//`/`/* */` comments an AST round-trip cannot carry):",
        dropped.len()
    );
    for c in dropped.iter().take(10) {
        let first = c.lines().next().unwrap_or(c.as_str());
        eprintln!("    {first}");
    }
    if dropped.len() > 10 {
        eprintln!("    … and {} more", dropped.len() - 10);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::spanned::Spanned;

    #[test]
    fn test_line_starts() {
        // "ab\ncd\n\nef" -> lines start at 0, 3, 6, 7
        let sm = SourceMap::new("ab\ncd\n\nef");
        assert_eq!(sm.line_starts, vec![0, 3, 6, 7]);
    }

    #[test]
    fn test_line_col_to_byte_ascii() {
        let src = "abc\ndef\n";
        let sm = SourceMap::new(src);
        // line 1 col 0 -> 0 ; line 1 col 2 -> 2 ; line 2 col 0 -> 4 ; line 2 col 1 -> 5
        assert_eq!(sm.line_col_to_byte(1, 0), Some(0));
        assert_eq!(sm.line_col_to_byte(1, 2), Some(2));
        assert_eq!(sm.line_col_to_byte(2, 0), Some(4));
        assert_eq!(sm.line_col_to_byte(2, 1), Some(5));
        assert_eq!(sm.line_col_to_byte(0, 0), None);
        assert_eq!(sm.line_col_to_byte(99, 0), None);
    }

    #[test]
    fn test_line_col_to_byte_multibyte() {
        // line 2 is "αβ" — each Greek letter is 2 bytes in UTF-8.
        let src = "a\nαβ\nz";
        let sm = SourceMap::new(src);
        // line 2 starts after "a\n" = byte 2.
        // col 0 -> byte 2 ; col 1 -> byte 2 + 2 = 4 ; col 2 -> byte 6 (end of αβ).
        assert_eq!(sm.line_col_to_byte(2, 0), Some(2));
        assert_eq!(sm.line_col_to_byte(2, 1), Some(4));
        assert_eq!(sm.line_col_to_byte(2, 2), Some(6));
        // verify the slice is exactly "αβ"
        let (a, b) = (
            sm.line_col_to_byte(2, 0).expect("start"),
            sm.line_col_to_byte(2, 2).expect("end"),
        );
        assert_eq!(&src[a..b], "αβ");
    }

    #[test]
    fn test_item_verbatim_preserves_doc_and_inner_comment() {
        let src = "/// outer doc\npub fn f() {\n    // inner note\n    let x = 1;\n}\n";
        let item: syn::ItemFn = syn::parse_str(src).expect("parse ItemFn");
        let sm = SourceMap::new(src);
        let slice = sm
            .item_verbatim(item.span(), &item.attrs)
            .expect("verbatim slice");
        assert!(slice.contains("/// outer doc"), "missing doc: {slice:?}");
        assert!(slice.contains("// inner note"), "missing inner: {slice:?}");
        assert!(slice.contains("let x = 1;"), "missing body: {slice:?}");
    }

    #[test]
    fn test_item_verbatim_with_indent_preserves_indentation() {
        // The fn is indented 4 spaces. parse_str on a single indented fn keeps
        // spans relative to THIS string, so the indent precedes col-of-`pub`.
        let src = "    pub fn f() {\n        let x = 1;\n    }\n";
        let item: syn::ItemFn = syn::parse_str(src).expect("parse ItemFn");
        let sm = SourceMap::new(src);
        let with_indent = sm
            .item_verbatim_with_indent(item.span(), &item.attrs)
            .expect("indented slice");
        assert!(
            with_indent.starts_with("    pub fn f()"),
            "indentation not preserved: {with_indent:?}"
        );
        // The non-indent variant should NOT start with spaces.
        let no_indent = sm.item_verbatim(item.span(), &item.attrs).expect("slice");
        assert!(no_indent.starts_with("pub fn f()"), "got: {no_indent:?}");
    }

    #[test]
    fn test_span_slice_exact() {
        let src = "pub fn g() {}\n";
        let item: syn::ItemFn = syn::parse_str(src).expect("parse");
        let sm = SourceMap::new(src);
        let slice = sm.span_slice(item.span()).expect("slice");
        assert_eq!(slice, "pub fn g() {}");
    }

    #[test]
    fn test_impl_header_verbatim() {
        let src = "impl Foo {\n    fn a(&self) {}\n}\n";
        let item: syn::ItemImpl = syn::parse_str(src).expect("parse impl");
        let sm = SourceMap::new(src);
        let header = sm.impl_header_verbatim(&item).expect("header");
        assert_eq!(header, "impl Foo {");
    }

    #[test]
    fn test_item_cut_range_removes_item_keeps_surroundings() {
        let src = "pub fn a() {}\n\n// keep me\n#[cfg(test)]\nmod t {\n    // inner\n}\n\npub fn b() {}\n";
        let file: syn::File = syn::parse_file(src).expect("parse file");
        // Second item is the `mod t`.
        let mod_item = file
            .items
            .iter()
            .find_map(|it| match it {
                syn::Item::Mod(m) => Some(m),
                _ => None,
            })
            .expect("mod item");
        let sm = SourceMap::new(src);
        let (start, end) = sm
            .item_cut_range(mod_item.span(), &mod_item.attrs)
            .expect("cut range");
        let cut = format!("{}mod t;\n{}", &src[..start], &src[end..]);
        // The whole `#[cfg(test)] mod t { … }` block is gone, replaced verbatim.
        assert!(cut.contains("pub fn a() {}"), "kept preceding fn: {cut:?}");
        assert!(cut.contains("pub fn b() {}"), "kept following fn: {cut:?}");
        assert!(
            cut.contains("// keep me"),
            "kept preceding comment: {cut:?}"
        );
        assert!(cut.contains("mod t;"), "spliced declaration: {cut:?}");
        assert!(!cut.contains("// inner"), "removed inner body: {cut:?}");
        assert!(!cut.contains("mod t {"), "removed inline body: {cut:?}");
    }

    #[test]
    fn test_extract_comment_texts_excludes_docs_and_string_lookalikes() {
        let src = r##"
/// doc line
//! inner doc
// real comment
pub fn f<'a>(x: &'a str) -> &'a str {
    let s = "not // a comment";
    let r = r#"also /* not */ a comment"#;
    let c = '/'; // trailing real comment
    /* block comment */
    x
}
"##;
        let comments = extract_comment_texts(src);
        assert!(
            comments.iter().any(|c| c == "// real comment"),
            "missing real comment: {comments:?}"
        );
        assert!(
            comments.iter().any(|c| c == "// trailing real comment"),
            "missing trailing comment: {comments:?}"
        );
        assert!(
            comments.iter().any(|c| c == "/* block comment */"),
            "missing block comment: {comments:?}"
        );
        // Doc comments are excluded.
        assert!(
            !comments.iter().any(|c| c.contains("doc line")),
            "doc comment leaked: {comments:?}"
        );
        assert!(
            !comments.iter().any(|c| c.contains("inner doc")),
            "inner doc leaked: {comments:?}"
        );
        // `//` inside a string / raw string / char literal is not a comment.
        assert!(
            !comments.iter().any(|c| c.contains("not // a comment")),
            "string content misread as comment: {comments:?}"
        );
        assert!(
            !comments.iter().any(|c| c.contains("also /* not */")),
            "raw-string content misread as comment: {comments:?}"
        );
    }

    #[test]
    fn test_dropped_comment_texts_detects_loss_and_survival() {
        let input = "// keep\n// drop me\npub fn f() {}\n";
        // Output preserves only the first comment.
        let output = "// keep\npub fn f() {}\n";
        let dropped = dropped_comment_texts(input, output);
        assert_eq!(
            dropped,
            vec!["// drop me".to_string()],
            "should flag the loss"
        );

        // When everything survives, nothing is reported.
        let full = "// keep\n// drop me\npub fn f() {}\n";
        assert!(
            dropped_comment_texts(input, full).is_empty(),
            "no loss expected when all comments survive"
        );
    }
}
