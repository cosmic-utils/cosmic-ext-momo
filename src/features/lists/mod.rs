//! User lists: browse the account's lists and view a selected list's timeline.

use cosmic::{
    app::Task,
    iced::{Alignment, Length, Subscription},
    widget, Apply, Element,
};
use megalodon::entities::List;

use crate::{
    app,
    cache::Cache,
    client::Client,
    features::timeline::{self, Timeline, TimelineKind},
};

pub struct Lists {
    mastodon: Client,
    lists: Vec<List>,
    loaded: bool,
    error: Option<String>,
    request: u64,
    selected: Option<Timeline>,
}

#[derive(Debug, Clone)]
pub enum Message {
    SetClient(Client),
    Retry,
    Failed(u64, String),
    SetLists(u64, Vec<List>),
    Select(String),
    Timeline(timeline::Message),
}

impl Lists {
    pub fn new(mastodon: Client) -> Self {
        Self {
            mastodon,
            lists: Vec::new(),
            loaded: false,
            error: None,
            request: 0,
            selected: None,
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.mastodon.is_authenticated()
    }

    pub fn view<'a>(&'a self, cache: &'a Cache) -> Element<'a, Message> {
        let spacing = cosmic::theme::active().cosmic().spacing;
        let lists: Vec<Element<_>> = self
            .lists
            .iter()
            .map(|list| {
                widget::button::suggested(list.title.clone())
                    .on_press(Message::Select(list.id.clone()))
                    .into()
            })
            .collect();

        let content = self
            .selected
            .as_ref()
            .map(|timeline| timeline.view(cache).map(Message::Timeline));

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
            widget::scrollable(widget::row(lists).spacing(spacing.space_xs)).direction(
                cosmic::iced::widget::scrollable::Direction::Horizontal(Default::default())
            ),
            content,
        ]
        .spacing(spacing.space_xs)
        .width(Length::Fill)
        .height(Length::Fill)
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
            Message::SetLists(request, lists) if request == self.request => {
                self.lists = lists;
                self.loaded = true;
                self.error = None;
            }
            Message::SetLists(..) | Message::Failed(..) => (),
            Message::Select(id) => {
                let mut timeline = Timeline::new(self.mastodon.clone(), TimelineKind::List(id));
                let task = timeline.load_cached();
                self.selected = Some(timeline);
                return task;
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
            subscriptions.push(fetch_lists(self.mastodon.clone(), self.request));
        }
        if let Some(timeline) = &self.selected {
            subscriptions.push(timeline.subscription().map(Message::Timeline));
        }
        Subscription::batch(subscriptions)
    }
}

fn fetch_lists(mastodon: Client, request: u64) -> Subscription<Message> {
    Subscription::run_with((mastodon, request), |(mastodon, request)| {
        let request = *request;
        let mastodon = mastodon.clone();
        cosmic::iced::stream::channel(
            1,
            move |mut output: futures_channel::mpsc::Sender<Message>| async move {
                use futures_util::SinkExt;
                match crate::client::within_timeout(
                    mastodon.get_lists(),
                    crate::fl!("feed-load-error"),
                )
                .await
                {
                    Ok(response) => {
                        if let Err(err) =
                            output.send(Message::SetLists(request, response.json)).await
                        {
                            tracing::warn!("failed to send lists: {}", err);
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
