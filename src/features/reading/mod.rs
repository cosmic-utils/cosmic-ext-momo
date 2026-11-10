//! Reading state shared by timelines, conversations, profiles, and search.
pub mod filters;
pub mod html;
pub mod profile;
pub mod report;
pub mod thread;

use megalodon::entities::Poll;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct State {
    pub post_menu: Option<String>,
    pub warnings: HashSet<String>,
    pub media: HashSet<String>,
    pub collapsed_replies: HashSet<String>,
    pub filter_reveals: HashSet<(String, filters::Context)>,
    pub filters: filters::Filters,
    pub polls: HashMap<String, Poll>,
    pub choices: HashMap<String, Vec<u32>>,
    pub poll_busy: HashSet<String>,
    pub poll_errors: HashMap<String, String>,
}
impl State {
    pub fn content_visible(&self, post: &megalodon::entities::Status) -> bool {
        post.spoiler_text.is_empty() || self.warnings.contains(&post.id)
    }
    pub fn media_visible(&self, post: &megalodon::entities::Status, filtered: bool) -> bool {
        self.content_visible(post)
            && ((!post.sensitive && !filtered) || self.media.contains(&post.id))
    }
    pub fn new(authenticated: bool) -> Self {
        Self {
            post_menu: None,
            warnings: HashSet::new(),
            media: HashSet::new(),
            collapsed_replies: HashSet::new(),
            filter_reveals: HashSet::new(),
            filters: if authenticated {
                filters::Filters::Loading
            } else {
                filters::Filters::ready(vec![])
            },
            polls: HashMap::new(),
            choices: HashMap::new(),
            poll_busy: HashSet::new(),
            poll_errors: HashMap::new(),
        }
    }
}

pub fn poll_open(poll: &Poll) -> bool {
    !poll.expired
        && !poll
            .expires_at
            .is_some_and(|expiry| expiry <= chrono::Utc::now())
}
pub fn valid_choices(poll: &Poll, choices: &[u32]) -> bool {
    !choices.is_empty()
        && (poll.multiple || choices.len() == 1)
        && choices
            .iter()
            .all(|choice| (*choice as usize) < poll.options.len())
        && choices
            .iter()
            .enumerate()
            .all(|(i, choice)| !choices[..i].contains(choice))
}

pub fn filter_notice(
    cache: &crate::cache::Cache,
) -> Option<cosmic::Element<'_, crate::features::status::Message>> {
    use cosmic::widget;
    match cache.reading.filters {
        filters::Filters::Loading => Some(
            widget::row![
                widget::indeterminate_circular().size(24.),
                widget::text(crate::fl!("reading-filters-loading"))
            ]
            .spacing(12)
            .into(),
        ),
        filters::Filters::Failed => Some(
            widget::column![
                widget::text(crate::fl!("reading-filters-error")),
                widget::button::standard(crate::fl!("reading-retry"))
                    .on_press(crate::features::status::Message::ReloadFilters)
            ]
            .spacing(12)
            .into(),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
