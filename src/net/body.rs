//! Bounded reads and character decoding of response bodies.
use encoding_rs::{Encoding, UTF_8};
use std::io::Read;

pub(super) fn limit_error(max_bytes: usize) -> String {
    format!("Resource exceeds the {max_bytes}-byte limit.")
}

pub(super) fn read_limit(reader: impl Read, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Could not read document: {e}"))?;
    if bytes.len() > max_bytes {
        return Err(limit_error(max_bytes));
    }
    Ok(bytes)
}

pub(super) fn decode_limit(
    bytes: &[u8],
    content_type: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    decode_with_encoding(bytes, declared_encoding(content_type), max_bytes)
}

pub(super) fn decode_html_limit(
    bytes: &[u8],
    content_type: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let declared = declared_encoding(content_type);
    let bom = Encoding::for_bom(bytes);
    let encoding = bom.map(|(encoding, _)| encoding).or(declared).or_else(|| {
        sniff_html_encoding(bytes).and_then(|label| {
            let encoding = Encoding::for_label(label.as_bytes())?;
            Some(
                if encoding == encoding_rs::UTF_16LE || encoding == encoding_rs::UTF_16BE {
                    UTF_8
                } else if encoding == encoding_rs::X_USER_DEFINED {
                    encoding_rs::WINDOWS_1252
                } else {
                    encoding
                },
            )
        })
    });
    decode_with_encoding(bytes, encoding, max_bytes)
}

fn declared_encoding(content_type: &str) -> Option<&'static Encoding> {
    content_type
        .split(';')
        .skip(1)
        .find_map(|part| {
            let (name, value) = part.trim().split_once('=')?;
            name.trim()
                .eq_ignore_ascii_case("charset")
                .then(|| value.trim().trim_matches(['\'', '"']))
        })
        .and_then(|label| Encoding::for_label(label.as_bytes()))
}

fn decode_with_encoding(
    bytes: &[u8],
    declared: Option<&'static Encoding>,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let (encoding, skip) = Encoding::for_bom(bytes).unwrap_or((declared.unwrap_or(UTF_8), 0));
    let (decoded, _) = encoding.decode_without_bom_handling(&bytes[skip..]);
    if decoded.len() > max_bytes {
        return Err(limit_error(max_bytes));
    }
    Ok(decoded.into_owned().into_bytes())
}

/// Prescan the first 1024 bytes for the HTML encoding declarations accepted by
/// browsers. It deliberately only recognizes `<meta>` attributes and never
/// treats visible text as a declaration.
fn sniff_html_encoding(bytes: &[u8]) -> Option<String> {
    let end = bytes.len().min(1024);
    let bytes = &bytes[..end];
    let mut cursor = 0;
    while let Some(relative) = bytes[cursor..].iter().position(|byte| *byte == b'<') {
        let start = cursor + relative;
        cursor = start + 1;
        if bytes
            .get(cursor..)
            .is_some_and(|rest| rest.starts_with(b"!--"))
        {
            cursor = bytes[cursor + 3..]
                .windows(3)
                .position(|window| window == b"-->")
                .map_or(end, |offset| cursor + 3 + offset + 3);
            continue;
        }
        let mut quote = None;
        let Some(tag_end) = bytes[start..].iter().position(|&byte| {
            if let Some(delimiter) = quote {
                if byte == delimiter {
                    quote = None;
                }
            } else if matches!(byte, b'\'' | b'"') {
                quote = Some(byte);
            } else if byte == b'>' {
                return true;
            }
            false
        }) else {
            break;
        };
        let tag = &bytes[start + 1..start + tag_end];
        let mut name_end = 0;
        while name_end < tag.len() && tag[name_end].is_ascii_alphabetic() {
            name_end += 1;
        }
        if !tag[..name_end].eq_ignore_ascii_case(b"meta")
            || tag
                .get(name_end)
                .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'/')
        {
            cursor = start + tag_end + 1;
            continue;
        }
        let attributes = meta_attributes(&tag[name_end..]);
        if let Some(value) = attributes
            .iter()
            .find(|(name, _)| name == "charset")
            .map(|(_, value)| value.clone())
            .or_else(|| {
                if !attributes.iter().any(|(name, value)| {
                    name == "http-equiv" && value.eq_ignore_ascii_case("content-type")
                }) {
                    return None;
                }
                attributes
                    .iter()
                    .find(|(name, _)| name == "content")
                    .and_then(|(_, value)| charset_parameter(value))
            })
        {
            if Encoding::for_label(value.trim().as_bytes()).is_some() {
                return Some(value.trim().to_owned());
            }
        }
        cursor = start + tag_end + 1;
    }
    None
}

fn meta_attributes(bytes: &[u8]) -> Vec<(String, String)> {
    let mut attributes = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        while cursor < bytes.len() && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b'/')
        {
            cursor += 1;
        }
        let start = cursor;
        while cursor < bytes.len()
            && !bytes[cursor].is_ascii_whitespace()
            && !matches!(bytes[cursor], b'=' | b'/')
        {
            cursor += 1;
        }
        if start == cursor {
            cursor += 1;
            continue;
        }
        let name = String::from_utf8_lossy(&bytes[start..cursor]).to_ascii_lowercase();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let mut value = String::new();
        if bytes.get(cursor) == Some(&b'=') {
            cursor += 1;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if let Some(quote @ (b'"' | b'\'')) = bytes.get(cursor).copied() {
                cursor += 1;
                let value_start = cursor;
                while cursor < bytes.len() && bytes[cursor] != quote {
                    cursor += 1;
                }
                value = String::from_utf8_lossy(&bytes[value_start..cursor]).into_owned();
                cursor += usize::from(cursor < bytes.len());
            } else {
                let value_start = cursor;
                while cursor < bytes.len() && !bytes[cursor].is_ascii_whitespace() {
                    cursor += 1;
                }
                value = String::from_utf8_lossy(&bytes[value_start..cursor]).into_owned();
            }
        }
        if !attributes.iter().any(|(existing, _)| existing == &name) {
            attributes.push((name, value));
        }
    }
    attributes
}

fn charset_parameter(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let start = lower.find("charset")? + "charset".len();
    let rest = &value[start..];
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest
        .strip_prefix('\'')
        .or_else(|| rest.strip_prefix('"'))
        .unwrap_or(rest);
    let end = rest
        .find(|character: char| {
            character.is_ascii_whitespace()
                || character == ';'
                || character == '\''
                || character == '"'
        })
        .unwrap_or(rest.len());
    (end > 0).then(|| rest[..end].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::MAX_DOCUMENT_BYTES;

    fn read_bounded(reader: impl Read) -> Result<Vec<u8>, String> {
        read_limit(reader, MAX_DOCUMENT_BYTES)
    }

    fn decode(bytes: &[u8], content_type: &str) -> Result<Vec<u8>, String> {
        decode_limit(bytes, content_type, MAX_DOCUMENT_BYTES)
    }

    #[test]
    fn bounded_decoding_handles_header_and_bom() {
        assert_eq!(
            decode(b"caf\xe9", "text/html; charset=windows-1252").unwrap(),
            "café".as_bytes()
        );
        assert_eq!(
            decode(
                b"\xef\xbb\xbfcaf\xc3\xa9",
                "text/html; charset=windows-1252"
            )
            .unwrap(),
            "café".as_bytes()
        );
        assert!(read_bounded(&vec![b'x'; MAX_DOCUMENT_BYTES + 1][..]).is_err());
        assert!(
            decode(
                &vec![0xe9; MAX_DOCUMENT_BYTES],
                "text/html;charset=windows-1252"
            )
            .is_err()
        );
    }

    #[test]
    fn html_meta_charset_is_sniffed_before_utf8_fallback() {
        let html = b"<html><head><meta charset='windows-1252'></head><body>caf\xe9</body>";
        let decoded = decode_html_limit(html, "text/html", MAX_DOCUMENT_BYTES).unwrap();
        assert_eq!(
            String::from_utf8(decoded).unwrap(),
            "<html><head><meta charset='windows-1252'></head><body>café</body>"
        );
        assert_eq!(
            sniff_html_encoding(
                b"<meta http-equiv=Content-Type content='text/html; charset=iso-8859-1'>"
            )
            .as_deref(),
            Some("iso-8859-1")
        );
        assert_eq!(sniff_html_encoding(b"<p>charset=windows-1252</p>"), None);
    }

    #[test]
    fn meta_charset_obeys_precedence_pragma_quotes_and_prescan_limit() {
        assert_eq!(sniff_html_encoding(b"<!-- <meta charset=windows-1252> --><meta content='text/html; charset=windows-1252'>"), None);
        assert_eq!(
            sniff_html_encoding(
                b"<meta2 charset=windows-1252><p data-x='<meta charset=windows-1252>'>"
            ),
            None
        );
        assert_eq!(
            sniff_html_encoding(b"<META data-x='>' CHARSET=windows-1252 charset=utf-8>").as_deref(),
            Some("windows-1252")
        );
        let mut html = vec![b' '; 1024];
        html.extend_from_slice(b"<meta charset=windows-1252>");
        assert_eq!(sniff_html_encoding(&html), None);
        let html = b"<meta charset=windows-1252>caf\xc3\xa9";
        assert_eq!(
            decode_html_limit(html, "text/html; charset=utf-8", MAX_DOCUMENT_BYTES).unwrap(),
            html
        );
        let mut bom = b"\xef\xbb\xbf".to_vec();
        bom.extend_from_slice(html);
        assert_eq!(
            decode_html_limit(&bom, "text/html; charset=windows-1252", MAX_DOCUMENT_BYTES).unwrap(),
            html
        );
        let html = b"<meta charset=utf-16>caf\xc3\xa9";
        assert_eq!(
            decode_html_limit(html, "text/html", MAX_DOCUMENT_BYTES).unwrap(),
            html
        );
        let html = b"<meta charset=x-user-defined>\x80";
        assert!(
            String::from_utf8(decode_html_limit(html, "text/html", MAX_DOCUMENT_BYTES).unwrap())
                .unwrap()
                .ends_with('€')
        );
        assert!(
            decode_html_limit(b"<meta charset=windows-1252>\xe9\xe9", "text/html", 29).is_err()
        );
    }
}
