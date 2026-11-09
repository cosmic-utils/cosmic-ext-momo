//! Cursor-aware completion; selections and partial tokens are never overwritten.
use crate::{
    client::{within_timeout, Client},
    fl,
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
#[derive(Clone, Debug)]
pub struct Suggestion {
    pub label: String,
    pub value: String,
}
pub fn token(text: &str, line: usize, column: usize) -> Option<(Range<usize>, String)> {
    let mut offset = 0;
    let current = text.split_inclusive('\n').nth(line)?;
    for previous in text.split_inclusive('\n').take(line) {
        offset += previous.len();
    }
    let current = current.trim_end_matches(['\r', '\n']);
    let cursor = current
        .grapheme_indices(true)
        .nth(column)
        .map(|(byte, _)| byte)
        .unwrap_or(current.len());
    let allowed = |ch: char| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.' | '@' | '#');
    if current[cursor..].chars().next().is_some_and(allowed) {
        return None;
    }
    let start = current[..cursor]
        .char_indices()
        .rev()
        .find(|(_, ch)| !allowed(*ch))
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0);
    let query = &current[start..cursor];
    if query.graphemes(true).count() < 2 || !(query.starts_with('@') || query.starts_with('#')) {
        return None;
    }
    Some((offset + start..offset + cursor, query.to_owned()))
}
pub async fn fetch(client: Client, query: String) -> Vec<Suggestion> {
    let tag = query.starts_with('#');
    let options = megalodon::megalodon::SearchInputOptions {
        r#type: Some(if tag {
            megalodon::megalodon::SearchType::Hashtags
        } else {
            megalodon::megalodon::SearchType::Accounts
        }),
        limit: Some(5),
        resolve: Some(false),
        ..Default::default()
    };
    let Ok(response) = within_timeout(
        client.search(query[1..].to_owned(), Some(&options)),
        fl!("search-error"),
    )
    .await
    else {
        return vec![];
    };
    let suggestions = if tag {
        response
            .json
            .hashtags
            .into_iter()
            .map(|tag| Suggestion {
                label: format!("#{}", tag.name),
                value: format!("#{}", tag.name),
            })
            .collect::<Vec<_>>()
    } else {
        response
            .json
            .accounts
            .into_iter()
            .map(|account| {
                let value = crate::features::reading::html::full_acct(&account);
                Suggestion {
                    label: format!("{} {}", account.display_name, value),
                    value,
                }
            })
            .collect()
    };
    suggestions
        .into_iter()
        .filter(|item| !item.value.contains(['\n', '\r']))
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_uses_graphemes_and_keeps_surrounding_text() {
        let text = "Hello\n😀 @al next";
        let (range, query) = token(text, 1, 5).unwrap();
        assert_eq!(query, "@al");
        assert_eq!(&text[range.clone()], "@al");
        let mut completed = text.to_owned();
        completed.replace_range(range, "@alice@example.org ");
        assert_eq!(completed, "Hello\n😀 @alice@example.org  next");
        assert!(token("email@example.org", 0, 17).is_none());
        assert!(token("@alice", 0, 3).is_none());
    }
}
