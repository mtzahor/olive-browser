use crate::{Document, sink::Sink};
use html5ever::tendril::TendrilSink;
use std::{
    fmt,
    io::{self, Read},
};

/// Resource limits applied before parsing. These are not a CPU or heap sandbox.
#[derive(Clone, Copy, Debug)]
pub struct ParseOptions {
    /// Maximum encoded input size. Default: 1 MiB, including a possible UTF-8 BOM.
    pub max_input_bytes: usize,
    /// Maximum retained recovery diagnostics. Default: 64.
    pub max_diagnostics: usize,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            max_input_bytes: 1024 * 1024,
            max_diagnostics: 64,
        }
    }
}

/// A recoverable HTML/decoding error. The parser still produces a document.
#[derive(Debug)]
pub struct Diagnostic {
    /// Approximate parser line, not an exact source span.
    pub line: u64,
    pub message: String,
}

#[derive(Debug)]
pub struct ParseOutput {
    pub document: Document,
    pub diagnostics: Vec<Diagnostic>,
    pub omitted_diagnostics: usize,
}

/// Fatal input errors; malformed HTML itself is recovered according to HTML rules.
#[derive(Debug)]
pub enum ParseError {
    InputTooLarge { limit: usize },
    Io(io::Error),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputTooLarge { limit } => write!(f, "HTML input exceeds the {limit}-byte limit"),
            Self::Io(error) => write!(f, "could not read HTML input: {error}"),
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// Parse a UTF-8 document with default limits and scripting disabled.
pub fn parse(input: &str) -> Result<ParseOutput, ParseError> {
    parse_utf8(input.as_bytes(), ParseOptions::default())
}

/// Parse UTF-8 bytes. A leading BOM is removed, and invalid UTF-8 is replaced
/// with U+FFFD by html5ever's decoder. Legacy encoding sniffing is not implemented.
pub fn parse_utf8(input: &[u8], options: ParseOptions) -> Result<ParseOutput, ParseError> {
    if input.len() > options.max_input_bytes {
        return Err(ParseError::InputTooLarge {
            limit: options.max_input_bytes,
        });
    }
    let input = input.strip_prefix(b"\xef\xbb\xbf").unwrap_or(input);
    let mut parser_options = html5ever::ParseOpts::default();
    parser_options.tree_builder.scripting_enabled = false;
    Ok(
        html5ever::parse_document(Sink::new(options.max_diagnostics), parser_options)
            .from_utf8()
            .one(input),
    )
}

/// Read at most `max_input_bytes + 1` bytes, then parse the buffered UTF-8 document.
/// Reader errors are propagated without returning a partial document.
pub fn parse_reader(reader: impl Read, options: ParseOptions) -> Result<ParseOutput, ParseError> {
    let mut bytes = Vec::new();
    reader
        .take((options.max_input_bytes as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(ParseError::Io)?;
    parse_utf8(&bytes, options)
}
