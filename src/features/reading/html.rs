//! Render sanitized server HTML as selectable text runs and inline links.
use cosmic::{widget, Element};
use html2text::render::{RichAnnotation, TaggedLineElement};

#[derive(Debug, Clone, PartialEq)]
pub struct Fragment {
    pub text: String,
    pub url: Option<String>,
}

pub fn document(html: &str) -> Vec<Vec<Fragment>> {
    match html2text::config::rich()
        .link_footnotes(false)
        .lines_from_read(html.as_bytes(), 10000)
    {
        Ok(lines) => lines
            .iter()
            .map(|line| {
                line.iter()
                    .filter_map(|element| {
                        let TaggedLineElement::Str(text) = element else {
                            return None;
                        };
                        Some(Fragment {
                            text: text.s.clone(),
                            url: text.tag.iter().find_map(|tag| {
                                if let RichAnnotation::Link(url) = tag {
                                    web_url(url).then(|| url.clone())
                                } else {
                                    None
                                }
                            }),
                        })
                    })
                    .collect()
            })
            .collect(),
        Err(_) => vec![vec![Fragment {
            text: crate::fl!("reading-unavailable"),
            url: None,
        }]],
    }
}

pub fn plain(html: &str) -> String {
    document(html)
        .iter()
        .map(|line| {
            line.iter()
                .map(|part| part.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn web_url(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        matches!(url.scheme(), "https" | "http")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

pub fn full_acct(account: &megalodon::entities::Account) -> String {
    if account.acct.contains('@') {
        return format!("@{}", account.acct);
    }
    let host = reqwest::Url::parse(&account.url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned));
    match host {
        Some(host) => format!("@{}@{}", account.acct, host),
        None => format!("@{}", account.acct),
    }
}

pub fn view<'a, M: Clone + 'static>(
    html: &str,
    link: impl Fn(String) -> M + Copy,
) -> Element<'a, M> {
    let lines: Vec<Element<'a, M>> = document(html)
        .into_iter()
        .map(|line| {
            let runs: Vec<Element<'a, M>> = line
                .into_iter()
                .map(|part| {
                    if let Some(url) = part.url {
                        widget::button::link(part.text)
                            .padding(0)
                            .on_press(link(url))
                            .into()
                    } else {
                        widget::selectable_text(part.text).into()
                    }
                })
                .collect();
            if runs.is_empty() {
                widget::space::vertical().height(8.).into()
            } else {
                widget::flex_row(runs).into()
            }
        })
        .collect();
    widget::column(lines).into()
}
