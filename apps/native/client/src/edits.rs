//! Message edits: what an edited text must be.

pub fn valid_text(text: &str) -> bool {
    !text.trim().is_empty()
        && text.chars().count() <= 4000
        && !text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}
