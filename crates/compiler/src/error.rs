//! Errors reported to callers, with Mermaid-compatible messages and UTF-16 source positions.

/// The kind of failure, mirroring Pele's `PeleErrorCode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    /// A configured limit was exceeded.
    Limit,
    /// The source parsed but describes something invalid.
    Semantic,
    /// The source does not match the diagram's grammar.
    Syntax,
    /// The options ask for something that this compiler module does not support.
    Unsupported,
    /// No supported diagram type was detected.
    UnsupportedDiagram,
}

impl Code {
    /// The code as the TypeScript API reports it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Limit => "limit",
            Self::Semantic => "semantic",
            Self::Syntax => "syntax",
            Self::Unsupported => "unsupported",
            Self::UnsupportedDiagram => "unsupported-diagram",
        }
    }
}

/// A compile error. Messages are public contract and match Pele's wording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// The kind of failure.
    pub code: Code,
    /// One-based UTF-16 column, or 0 when the error has no position.
    pub column: u32,
    /// One-based line, or 0 when the error has no position.
    pub line: u32,
    /// The human-readable message.
    pub message: String,
    /// The offending source line with a caret under the column, or empty.
    pub snippet: String,
    /// The detected diagram type, when known.
    pub r#type: Option<&'static str>,
}

impl Error {
    /// Creates an error without a source position.
    #[cold]
    pub(crate) fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            column: 0,
            line: 0,
            message: message.into(),
            snippet: String::new(),
            r#type: None,
        }
    }

    /// Creates a syntax error at a UTF-8 byte `offset` of `src`, as Pele's `syntaxError` does.
    /// Lexical errors report unrecognized text and omit `detail`.
    #[cold]
    pub(crate) fn syntax(
        r#type: &'static str,
        src: &str,
        offset: usize,
        detail: &str,
        lexical: bool,
    ) -> Self {
        let location = locate(src, offset);
        let mut message = String::new();

        if lexical {
            message.push_str("Lexical error on line ");
            push_u32(&mut message, location.line);
            message.push_str(". Unrecognized text.\n");
            message.push_str(&location.snippet);
        } else {
            message.push_str("Parse error on line ");
            push_u32(&mut message, location.line);
            message.push_str(":\n");
            message.push_str(&location.snippet);
            message.push('\n');
            message.push_str(detail);
        }

        Self {
            code: Code::Syntax,
            column: location.column,
            line: location.line,
            message,
            snippet: location.snippet,
            r#type: Some(r#type),
        }
    }
}

/// A source position for diagnostics.
pub(crate) struct Location {
    pub(crate) column: u32,
    pub(crate) line: u32,
    pub(crate) snippet: String,
}

/// Finds the line, UTF-16 column, and caret snippet for a UTF-8 byte offset.
pub(crate) fn locate(src: &str, offset: usize) -> Location {
    let offset = floor_boundary(src, offset.min(src.len()));
    let start = src[..offset].rfind('\n').map_or(0, |at| at + 1);
    let end = src[offset..].find('\n').map_or(src.len(), |at| offset + at);
    let line = u32::try_from(src[..offset].matches('\n').count() + 1).unwrap_or(u32::MAX);
    let width = src[start..offset].encode_utf16().count();

    let mut snippet = String::with_capacity(end - start + width + 2);
    snippet.push_str(&src[start..end]);
    snippet.push('\n');
    for _ in 0..width {
        snippet.push('-');
    }
    snippet.push('^');

    Location {
        column: u32::try_from(width + 1).unwrap_or(u32::MAX),
        line,
        snippet,
    }
}

fn floor_boundary(src: &str, mut offset: usize) -> usize {
    while !src.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Appends a decimal integer without `core::fmt`.
pub(crate) fn push_u32(out: &mut String, mut value: u32) {
    let mut digits = [0u8; 10];
    let mut at = digits.len();
    loop {
        at -= 1;
        digits[at] = b'0' + u8::try_from(value % 10).unwrap_or(0);
        value /= 10;
        if value == 0 {
            break;
        }
    }
    for &digit in &digits[at..] {
        out.push(char::from(digit));
    }
}
