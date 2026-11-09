use chrono::{DateTime, Utc};
use megalodon::entities::Status;
use regex::{Regex, RegexBuilder};
use serde::Deserialize;

use super::html;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Context {
    Home,
    Public,
    Notifications,
    Thread,
    Account,
}
impl Context {
    fn name(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Public => "public",
            Self::Notifications => "notifications",
            Self::Thread => "thread",
            Self::Account => "account",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Keyword {
    keyword: String,
    whole_word: bool,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FilterStatus {
    status_id: String,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Filter {
    pub title: String,
    context: Vec<String>,
    expires_at: Option<DateTime<Utc>>,
    filter_action: String,
    #[serde(default)]
    keywords: Vec<Keyword>,
    #[serde(default)]
    statuses: Vec<FilterStatus>,
}
#[derive(Debug, Clone)]
pub struct Rule {
    definition: Filter,
    matchers: Vec<Regex>,
}
#[derive(Debug, Clone)]
pub enum Filters {
    Loading,
    Failed,
    Ready(Vec<Rule>),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Warn(Vec<String>, Vec<String>),
    Hide,
    Blur(Vec<String>),
    Loading,
    Failed,
}

impl Filters {
    pub fn same_definitions(&self, definitions: &[Filter]) -> bool {
        match self {
            Self::Ready(rules) => {
                rules.len() == definitions.len()
                    && rules
                        .iter()
                        .zip(definitions)
                        .all(|(rule, definition)| rule.definition == *definition)
            }
            _ => false,
        }
    }
    pub fn ready(definitions: Vec<Filter>) -> Self {
        Self::Ready(
            definitions
                .into_iter()
                .map(|definition| {
                    let matchers = definition
                        .keywords
                        .iter()
                        .filter(|keyword| !keyword.keyword.is_empty())
                        .filter_map(|keyword| {
                            let literal = regex::escape(&keyword.keyword);
                            let pattern = if keyword.whole_word {
                                format!(r"(?:^|\W){literal}(?:$|\W)")
                            } else {
                                literal
                            };
                            RegexBuilder::new(&pattern)
                                .case_insensitive(true)
                                .build()
                                .ok()
                        })
                        .collect();
                    Rule {
                        definition,
                        matchers,
                    }
                })
                .collect(),
        )
    }

    pub fn decision(&self, status: &Status, context: Context) -> Decision {
        let rules = match self {
            Self::Loading => return Decision::Loading,
            Self::Failed => return Decision::Failed,
            Self::Ready(rules) => rules,
        };
        if rules.is_empty() {
            return Decision::Allow;
        }
        let status = status.reblog.as_deref().unwrap_or(status);
        let mut text = format!("{}\n{}", status.spoiler_text, html::plain(&status.content));
        for media in &status.media_attachments {
            if let Some(description) = &media.description {
                text.push('\n');
                text.push_str(description);
            }
        }
        if let Some(poll) = &status.poll {
            for option in &poll.options {
                text.push('\n');
                text.push_str(&option.title);
            }
        }
        let mut warnings = vec![];
        let mut blurs = vec![];
        for rule in rules {
            let filter = &rule.definition;
            if !filter.context.iter().any(|value| value == context.name())
                || filter
                    .expires_at
                    .is_some_and(|expires| expires <= Utc::now())
            {
                continue;
            }
            if !filter
                .statuses
                .iter()
                .any(|entry| entry.status_id == status.id)
                && !rule.matchers.iter().any(|matcher| matcher.is_match(&text))
            {
                continue;
            }
            match filter.filter_action.as_str() {
                "hide" => return Decision::Hide,
                "blur" => blurs.push(filter.title.clone()),
                _ => warnings.push(filter.title.clone()),
            }
        }
        if !warnings.is_empty() {
            Decision::Warn(warnings, blurs)
        } else if !blurs.is_empty() {
            Decision::Blur(blurs)
        } else {
            Decision::Allow
        }
    }
}

pub async fn fetch(client: crate::client::Client) -> Result<Vec<Filter>, String> {
    if !client.is_authenticated() {
        return Ok(vec![]);
    }
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| crate::fl!("reading-filters-error"))?;
    let response = http
        .get(format!(
            "{}/api/v2/filters",
            client.base_url.trim_end_matches('/')
        ))
        .bearer_auth(client.token.as_deref().unwrap_or_default())
        .send()
        .await
        .map_err(|_| crate::fl!("reading-filters-error"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        let legacy = tokio::time::timeout(std::time::Duration::from_secs(30), client.get_filters())
            .await
            .ok()
            .and_then(Result::ok)
            .ok_or_else(|| crate::fl!("reading-filters-error"))?;
        return Ok(legacy
            .json
            .into_iter()
            .map(|filter| Filter {
                title: filter.phrase.clone(),
                context: filter
                    .context
                    .iter()
                    .map(|context| {
                        match context {
                            megalodon::entities::filter::FilterContext::Home => "home",
                            megalodon::entities::filter::FilterContext::Notifications => {
                                "notifications"
                            }
                            megalodon::entities::filter::FilterContext::Public => "public",
                            megalodon::entities::filter::FilterContext::Thread => "thread",
                        }
                        .to_string()
                    })
                    .collect(),
                expires_at: filter.expires_at,
                filter_action: if filter.irreversible { "hide" } else { "warn" }.into(),
                keywords: vec![Keyword {
                    keyword: filter.phrase,
                    whole_word: filter.whole_word,
                }],
                statuses: vec![],
            })
            .collect());
    }
    let bytes = response
        .error_for_status()
        .map_err(|_| crate::fl!("reading-filters-error"))?
        .bytes()
        .await
        .map_err(|_| crate::fl!("reading-filters-error"))?;
    serde_json::from_slice(&bytes).map_err(|_| crate::fl!("reading-filters-error"))
}
