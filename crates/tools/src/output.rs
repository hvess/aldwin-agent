//! How much of a tool's output reaches the model.

/// Bytes of output a tool hands the model at once: all of it stays in the
/// conversation for the rest of the session, and is sent again every step.
pub(crate) const OUTPUT_CAP_BYTES: usize = 50 * 1024;

/// `text` as far as the last character boundary within `OUTPUT_CAP_BYTES`.
pub(crate) fn within_cap(text: &str) -> &str {
    if text.len() <= OUTPUT_CAP_BYTES {
        return text;
    }
    let mut end = OUTPUT_CAP_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// `text` cut to `OUTPUT_CAP_BYTES`, with a line saying it was.
pub(crate) fn capped(text: &str) -> String {
    let kept = within_cap(text);
    if kept.len() == text.len() {
        return text.to_string();
    }
    format!("{kept}\n[truncated at {OUTPUT_CAP_BYTES} bytes]\n")
}
