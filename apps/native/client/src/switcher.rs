//! Keyboard navigation shared by the quick switcher (Ctrl+K) and the sidebar's
//! Alt+Up/Down stepping. Every client implements the same matching and
//! stepping, checked against `shared/shortcuts/switcher-cases.json`.

fn word(character: char) -> bool {
    character.is_alphanumeric()
}

/// How well `query` matches `text`: higher is better, `None` for no match,
/// `Some(0)` for an empty query.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let query = query.trim();
    let query = query
        .strip_prefix('#')
        .or_else(|| query.strip_prefix('@'))
        .unwrap_or(query);
    let q: Vec<char> = query.to_lowercase().chars().collect();
    if q.is_empty() {
        return Some(0);
    }
    let lower = text.to_lowercase();
    let joined: String = q.iter().collect();
    if lower == joined {
        return Some(1000);
    }
    if lower.starts_with(&joined) {
        return Some(900);
    }
    let t: Vec<char> = lower.chars().collect();
    let word_start = |index: usize| index == 0 || !word(t[index - 1]);
    if (0..t.len()).any(|index| word_start(index) && t[index..].starts_with(&q)) {
        return Some(800);
    }
    if lower.contains(&joined) {
        return Some(700);
    }
    let mut score = 100i32;
    let mut last: Option<usize> = None;
    let mut from = 0;
    for &character in &q {
        let index = from + t.get(from..)?.iter().position(|&c| c == character)?;
        if last.is_some_and(|last| index == last + 1) {
            score += 10;
        }
        if word_start(index) {
            score += 5;
        }
        score -= (index - from) as i32;
        last = Some(index);
        from = index + 1;
    }
    Some(score.clamp(1, 699))
}

/// The indexes of the items that match, best first; unread ones lead among
/// equals. Each item is its name and alternates (such as a username) and
/// whether it is unread.
pub fn rank<'a, I, T>(items: I, query: &str) -> Vec<usize>
where
    I: IntoIterator<Item = (T, bool)>,
    T: IntoIterator<Item = &'a str>,
{
    let mut scored: Vec<(i32, usize)> = items
        .into_iter()
        .enumerate()
        .filter_map(|(index, (texts, unread))| {
            let best = texts
                .into_iter()
                .filter_map(|text| score(query, text))
                .max()?;
            Some((best + if unread { 50 } else { 0 }, index))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, index)| index).collect()
}

/// The conversation Alt+Up/Down (or, with `unread_only`, Alt+Shift+Up/Down)
/// moves to from `current`, wrapping; `None` when there is none.
pub fn adjacent(
    unread: &[bool],
    current: Option<usize>,
    forward: bool,
    unread_only: bool,
) -> Option<usize> {
    let count = unread.len();
    for step in 0..count {
        let index = match current {
            None if forward => step,
            None => count - 1 - step,
            Some(current) if forward => (current + step + 1) % count,
            Some(current) => (current + count * 2 - step - 1) % count,
        };
        if Some(index) == current {
            return None;
        }
        if !unread_only || unread[index] {
            return Some(index);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn cases() -> Value {
        serde_json::from_str(include_str!(
            "../../../../shared/shortcuts/switcher-cases.json"
        ))
        .unwrap()
    }

    #[test]
    fn matching_follows_the_shared_cases() {
        let cases = cases();
        for case in cases["match"].as_array().unwrap() {
            let expected = case["score"].as_i64().map(|score| score as i32);
            let (query, text) = (
                case["query"].as_str().unwrap(),
                case["text"].as_str().unwrap(),
            );
            assert_eq!(score(query, text), expected, "{query} ~ {text}");
        }
        for case in cases["rank"].as_array().unwrap() {
            let items: Vec<(Vec<&str>, bool)> = case["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| {
                    let mut texts = vec![item["name"].as_str().unwrap()];
                    texts.extend(
                        item["alternates"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|a| a.as_str().unwrap()),
                    );
                    (texts, item["unread"].as_bool().unwrap())
                })
                .collect();
            let order: Vec<usize> = case["order"]
                .as_array()
                .unwrap()
                .iter()
                .map(|index| index.as_u64().unwrap() as usize)
                .collect();
            assert_eq!(
                rank(items, case["query"].as_str().unwrap()),
                order,
                "{}",
                case["note"]
            );
        }
    }

    #[test]
    fn stepping_follows_the_shared_cases() {
        for case in cases()["adjacent"].as_array().unwrap() {
            let unread: Vec<bool> = case["unread"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_bool().unwrap())
                .collect();
            let current = case["current"].as_u64().map(|v| v as usize);
            let result = case["result"].as_u64().map(|v| v as usize);
            assert_eq!(
                adjacent(
                    &unread,
                    current,
                    case["forward"].as_bool().unwrap(),
                    case["unreadOnly"].as_bool().unwrap()
                ),
                result,
                "{case}"
            );
        }
    }
}
