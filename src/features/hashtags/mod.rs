//! Followed hashtags: browse followed tags, follow/unfollow them, and view a
//! selected tag's timeline.

use std::collections::HashSet;

use cosmic::{
    app::Task,
    iced::{Alignment, Length, Subscription},
    widget, Apply, Element,
};
use megalodon::entities::Tag;

use crate::{
    app,
    cache::Cache,
    client::Client,
    features::timeline::{self, Timeline, TimelineKind},
};

pub struct Hashtags {
    mastodon: Client,
    followed: Vec<Tag>,
    loaded: bool,
    error: Option<String>,
    request: u64,
    /// Tags the user has unfollowed during this visit to the page. Kept
    /// visible (with an updated button) until the page is left and revisited,
    /// at which point the followed-tags list is refetched and they drop out.
    unfollowed: HashSet<String>,
    filter: String,
    selected: Option<Timeline>,
}

#[derive(Debug, Clone)]
pub enum Message {
    SetClient(Client),
    Retry,
    Failed(u64, String),
    /// The page became active again; refresh the followed list from the
    /// server so tags unfollowed earlier finally disappear.
    Refresh,
    SetFollowed(u64, Vec<Tag>),
    FilterChanged(String),
    Select(String),
    /// Deselect the current tag, returning to the followed-tags list.
    Deselect,
    /// Toggle following a tag: (name, currently following).
    ToggleFollow(String, bool),
    FollowResult(String, bool),
    Timeline(timeline::Message),
}

impl Hashtags {
    pub fn new(mastodon: Client) -> Self {
        Self {
            mastodon,
            followed: Vec::new(),
            loaded: false,
            error: None,
            request: 0,
            unfollowed: HashSet::new(),
            filter: String::new(),
            selected: None,
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.mastodon.is_authenticated()
    }

    pub fn view<'a>(&'a self, cache: &'a Cache) -> Element<'a, Message> {
        let spacing = cosmic::theme::active().cosmic().spacing;

        // Once a tag is selected, collapse the list and show only its feed
        // (with a way back), rather than showing both at once.
        if let Some(timeline) = &self.selected {
            return widget::column![
                widget::button::text(crate::fl!("reading-back"))
                    .leading_icon(widget::icon::from_name("go-previous-symbolic"))
                    .on_press(Message::Deselect),
                timeline.view(cache).map(Message::Timeline),
            ]
            .spacing(spacing.space_xs)
            .width(Length::Fill)
            .height(Length::Fill)
            .apply(widget::container)
            .max_width(700)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Start)
            .into();
        }

        let filter_input = widget::text_input(crate::fl!("tags-filter"), &self.filter)
            .on_input(Message::FilterChanged);

        let filter = self.filter.to_lowercase();
        let section = self
            .followed
            .iter()
            .filter(|tag| filter.is_empty() || tag.name.to_lowercase().contains(&filter))
            .fold(
                widget::settings::section().title(crate::fl!("tags-followed")),
                |section, tag| {
                    let following = !self.unfollowed.contains(&tag.name);
                    section.add(widget::settings::item_row(vec![
                        widget::button::link(format!("#{}", tag.name))
                            .on_press(Message::Select(tag.name.clone()))
                            .into(),
                        widget::space::horizontal().into(),
                        widget::button::standard(if following {
                            crate::fl!("action-unfollow")
                        } else {
                            crate::fl!("action-follow")
                        })
                        .on_press(Message::ToggleFollow(tag.name.clone(), following))
                        .into(),
                    ]))
                },
            );

        let notice: Option<Element<'_, Message>> = self.error.as_ref().map(|error| {
            widget::column![
                widget::text(error),
                widget::button::standard(crate::fl!("reading-retry")).on_press(Message::Retry)
            ]
            .spacing(8)
            .into()
        });
        widget::column![
            notice,
            filter_input,
            widget::scrollable(section)
                .width(Length::Fill)
                .height(Length::Fill),
        ]
        .spacing(spacing.space_xs)
        .apply(widget::container)
        .max_width(700)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Start)
        .into()
    }

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        match message {
            Message::Retry => {
                self.request += 1;
                self.loaded = false;
                self.error = None;
            }
            Message::Failed(request, error) if request == self.request => {
                self.loaded = true;
                self.error = Some(error);
            }
            Message::SetClient(mastodon) => {
                self.mastodon = mastodon.clone();
                if let Some(timeline) = &mut self.selected {
                    return timeline.update(timeline::Message::SetClient(mastodon));
                }
            }
            Message::Refresh => {
                self.request += 1;
                self.error = None;
                self.loaded = false;
                self.unfollowed.clear();
            }
            Message::SetFollowed(request, tags) if request == self.request => {
                self.followed = tags;
                self.loaded = true;
                self.error = None;
            }
            Message::FilterChanged(filter) => self.filter = filter,
            Message::SetFollowed(..) | Message::Failed(..) => (),
            Message::Select(name) => {
                let mut timeline = Timeline::new(self.mastodon.clone(), TimelineKind::Tag(name));
                let task = timeline.load_cached();
                self.selected = Some(timeline);
                return task;
            }
            Message::Deselect => self.selected = None,
            Message::ToggleFollow(name, following) => {
                let mastodon = self.mastodon.clone();
                return cosmic::task::future(async move {
                    let result = crate::client::within_timeout(
                        async {
                            if following {
                                mastodon.unfollow_tag(name.clone()).await
                            } else {
                                mastodon.follow_tag(name.clone()).await
                            }
                        },
                        crate::fl!("action-error"),
                    )
                    .await;
                    match result {
                        Ok(response) => app::Message::Hashtags(Message::FollowResult(
                            name,
                            response.json.following.unwrap_or(!following),
                        )),
                        Err(err) => app::Message::Error(err),
                    }
                });
            }
            Message::FollowResult(name, following) => {
                if following {
                    self.unfollowed.remove(&name);
                } else {
                    self.unfollowed.insert(name);
                }
            }
            Message::Timeline(message) => {
                if let Some(timeline) = &mut self.selected {
                    return timeline.update(message);
                }
            }
        }
        Task::none()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![];
        if self.is_authenticated() && !self.loaded {
            subscriptions.push(fetch_followed_tags(self.mastodon.clone(), self.request));
        }
        if let Some(timeline) = &self.selected {
            subscriptions.push(timeline.subscription().map(Message::Timeline));
        }
        Subscription::batch(subscriptions)
    }
}

fn fetch_followed_tags(mastodon: Client, request: u64) -> Subscription<Message> {
    Subscription::run_with((mastodon, request), |(mastodon, request)| {
        let request = *request;
        let mastodon = mastodon.clone();
        cosmic::iced::stream::channel(
            1,
            move |mut output: futures_channel::mpsc::Sender<Message>| async move {
                use futures_util::SinkExt;
                match crate::client::within_timeout(
                    mastodon.get_followed_tags(),
                    crate::fl!("feed-load-error"),
                )
                .await
                {
                    Ok(response) => {
                        if let Err(err) = output
                            .send(Message::SetFollowed(request, response.json))
                            .await
                        {
                            tracing::warn!("failed to send followed tags: {}", err);
                        }
                    }
                    Err(error) => {
                        let _ = output.send(Message::Failed(request, error)).await;
                    }
                }
                std::future::pending().await
            },
        )
    })
}
