/// The longest prefix of `text` that fits in `max_bytes` without splitting a character.
pub(crate) fn truncate(text: &str, max_bytes: usize) -> &str {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
