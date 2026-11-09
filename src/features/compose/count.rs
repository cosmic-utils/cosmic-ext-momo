//! Count extended grapheme clusters, reserving server-defined URL lengths
//! and counting remote mentions as their local @username portion.
use regex::{Captures, Regex};
use std::sync::LazyLock;
use unicode_segmentation::UnicodeSegmentation;

pub fn count(text: &str, warning: &str, reserved_url: usize) -> usize {
    static URL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)https?://[^\s<>\"']+"#).unwrap());
    static MENTION: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(^|[^\w/@])@([a-z0-9_]+)@(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?::[0-9]+)?").unwrap()
    });
    let urls = URL.replace_all(text, |captures: &Captures<'_>| {
        let value = &captures[0];
        let mut end = value.len();
        while let Some(ch) = value[..end].chars().next_back() {
            let unmatched = match ch {
                ')' => value[..end].matches(')').count() > value[..end].matches('(').count(),
                ']' => value[..end].matches(']').count() > value[..end].matches('[').count(),
                _ => false,
            };
            if unmatched || ".,!?;:…".contains(ch) {
                end -= ch.len_utf8();
            } else {
                break;
            }
        }
        if end == 0 || !super::super::reading::html::web_url(&value[..end]) {
            return value.to_string();
        }
        format!("{}{}", "x".repeat(reserved_url), &value[end..])
    });
    let mentions = MENTION.replace_all(&urls, |captures: &Captures<'_>| {
        format!("{}@{}", &captures[1], &captures[2])
    });
    format!("{warning}{mentions}").graphemes(true).count()
}
