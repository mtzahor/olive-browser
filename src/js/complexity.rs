use super::{ErrorKind, ScriptError};

#[derive(Default)]
struct Frame {
    delimiter: u8,
    marks: usize,
    recursive: usize,
}

/// Bound active expression paths, rather than the sum of all statements in a file.
/// Strings and comments carry data, not AST depth. Ambiguous slash goals and
/// template substitutions fall back to the original stricter guard; Boa still
/// performs all syntax validation. This is deliberately conservative, not a lexer.
pub(super) fn check_complexity(source: &str) -> Result<(), ScriptError> {
    let bytes = source.as_bytes();
    let mut frames = vec![Frame::default()];
    let (mut marks, mut recursive, mut i) = (0usize, 0usize, 0usize);
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'`' || bytes[i..].starts_with(b"<!--") || bytes[i..].starts_with(b"-->") {
            return conservative_check(source);
        }
        if b == b'/' {
            match bytes.get(i + 1) {
                Some(b'/') => {
                    i += 2;
                    // Unicode line separators also end JavaScript line comments.
                    while i < bytes.len()
                        && !matches!(bytes[i], b'\n' | b'\r')
                        && !bytes[i..].starts_with(&[0xe2, 0x80, 0xa8])
                        && !bytes[i..].starts_with(&[0xe2, 0x80, 0xa9])
                    {
                        i += 1;
                    }
                    continue;
                }
                Some(b'*') => {
                    i += 2;
                    while i + 1 < bytes.len() && &bytes[i..i + 2] != b"*/" {
                        i += 1;
                    }
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                _ => return conservative_check(source),
            }
        }
        let mut recursive_mark = 0;
        if b.is_ascii_alphabetic() || matches!(b, b'_' | b'$') {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'$'))
            {
                i += 1;
            }
            recursive_mark = usize::from(matches!(
                &source[start..i],
                "new" | "typeof" | "void" | "delete" | "await" | "yield" | "if" | "else" | "do"
            ));
        } else {
            i += 1;
            if b.is_ascii_punctuation() && !matches!(b, b'_' | b'$') {
                frames.last_mut().unwrap().marks += 1;
                marks += 1;
            }
            if matches!(b, b'\'' | b'"') {
                while i < bytes.len() {
                    let next = bytes[i];
                    i += 1;
                    if next == b'\\' {
                        i = (i + 1).min(bytes.len());
                    } else if next == b {
                        break;
                    }
                }
            } else {
                recursive_mark = usize::from(b"!~?:".contains(&b));
                match b {
                    b'(' | b'[' | b'{' => frames.push(Frame {
                        delimiter: b,
                        ..Frame::default()
                    }),
                    b')' | b']' | b'}' => {
                        let expected = match b {
                            b')' => b'(',
                            b']' => b'[',
                            _ => b'{',
                        };
                        if frames.last().unwrap().delimiter != expected {
                            return conservative_check(source);
                        }
                        let frame = frames.pop().unwrap();
                        marks -= frame.marks;
                        recursive -= frame.recursive;
                    }
                    b';' if matches!(frames.last().unwrap().delimiter, 0 | b'{') => {
                        // Keep control/prefix counts: `if(x);else if(x);...` nests
                        // even though each branch contains a statement boundary.
                        marks -= frames.last().unwrap().marks;
                        frames.last_mut().unwrap().marks = 0;
                    }
                    _ => {}
                }
            }
        }
        frames.last_mut().unwrap().recursive += recursive_mark;
        recursive += recursive_mark;
        if marks > 512 || frames.len() > 33 || recursive > 32 {
            return Err(ScriptError::new(
                ErrorKind::Limit,
                "JavaScript expression complexity limit exceeded (512 active marks / 32 delimiter levels or recursive markers)",
            ));
        }
    }
    Ok(())
}

// A conservative preview guard before entering Boa's recursive parser/compiler.
// Delimiters and punctuation inside strings/comments count too. This intentionally
// rejects some valid large programs; it is not a second JavaScript tokenizer.
fn conservative_check(source: &str) -> Result<(), ScriptError> {
    let mut marks = 0usize;
    let mut depth = 0usize;
    let mut recursive_marks = source
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '$')
        .filter(|word| {
            matches!(
                *word,
                "new" | "typeof" | "void" | "delete" | "await" | "yield" | "if" | "else" | "do"
            )
        })
        .take(33)
        .count();
    for byte in source.bytes() {
        if byte.is_ascii_punctuation() && byte != b'_' && byte != b'$' {
            marks += 1;
        }
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if b"!~?:".contains(&byte) {
            recursive_marks += 1;
        }
        if marks > 512 || depth > 32 || recursive_marks > 32 {
            return Err(ScriptError::new(
                ErrorKind::Limit,
                "JavaScript source complexity limit exceeded (512 punctuation marks / 32 delimiter levels or recursive markers)",
            ));
        }
    }
    Ok(())
}
