//! `@mention` contract v1 (`caper_client::mentions`): token grammar,
//! composer suggestions and insertion, and what a person pill's card shows.

pub use caper_client::mentions::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_and_mention_tokens_are_never_active_together() {
        for value in [":smile@al", "@al:smi", ":+1 @al", "@al :tom"] {
            let caret = value.chars().count();
            assert!(
                !(token(value, caret).is_some() && crate::emoji::token(value, caret).is_some()),
                "{value}"
            );
        }
    }
}
