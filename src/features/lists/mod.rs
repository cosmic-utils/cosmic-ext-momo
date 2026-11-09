//! User lists: browse the account's lists and view a selected list's timeline.

pub mod editor;

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
    editor: Option<editor::Editor>,
    editor_serial: u64,
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
    Manage(Option<String>),
    Editor(editor::Message),
    Retry,
    Failed(u64, String),
    SetLists(u64, Vec<List>),
    Select(String),
    Timeline(timeline::Message),
}

impl Lists {
    pub fn new(mastodon: Client) -> Self {
        Self {
            editor: None,
            editor_serial: 0,
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
        if let Some(editor) = &self.editor {
            return editor.view().map(|message| match message {
                app::Message::Lists(message) => message,
                _ => Message::Retry,
            });
        }
        let spacing = cosmic::theme::active().cosmic().spacing;
        let lists: Vec<Element<_>> = self
            .lists
            .iter()
            .map(|list| {
                widget::row![
                    widget::button::suggested(list.title.clone())
                        .on_press(Message::Select(list.id.clone())),
                    widget::button::standard(crate::fl!("list-edit"))
                        .on_press(Message::Manage(Some(list.id.clone())))
                ]
                .spacing(4)
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
            widget::button::suggested(crate::fl!("list-create")).on_press(Message::Manage(None)),
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
            Message::Manage(id) => {
                self.request += 1;
                self.loaded = true;
                self.editor_serial += 1;
                let title = id
                    .as_ref()
                    .and_then(|id| self.lists.iter().find(|list| &list.id == id))
                    .map(|list| list.title.clone())
                    .unwrap_or_default();
                let mut editor = editor::Editor::new(id, title, self.editor_serial);
                let task = editor.load(self.mastodon.clone(), false);
                self.editor_serial = editor.serial;
                self.editor = Some(editor);
                return task;
            }
            Message::Editor(editor::Message::Close) => {
                self.editor = None;
                self.request += 1;
                self.loaded = false;
            }
            Message::Editor(editor::Message::Saved(serial, Ok(list)))
                if self
                    .editor
                    .as_ref()
                    .is_some_and(|editor| editor.serial == serial) =>
            {
                if let Some(list) = list {
                    self.lists.retain(|old| old.id != list.id);
                    self.lists.push(list);
                } else if let Some(id) = self.editor.as_ref().and_then(|editor| editor.id.as_ref())
                {
                    self.lists.retain(|list| &list.id != id);
                    self.selected = None;
                }
                self.editor = None;
            }
            Message::Editor(message) => {
                if let Some(editor) = &mut self.editor {
                    let task = editor.update(message, self.mastodon.clone());
                    self.editor_serial = editor.serial;
                    return task;
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closing_and_reopening_an_editor_rejects_old_mutation_results() {
        let mut lists = Lists::new(Client::new("https://example.org".into(), None));
        let _ = lists.update(Message::Manage(None));
        let old = lists.editor.as_ref().unwrap().serial;
        let _ = lists.update(Message::Editor(editor::Message::Close));
        let _ = lists.update(Message::Manage(None));
        assert!(lists.editor.as_ref().unwrap().serial > old);
        let result =
            serde_json::from_value(serde_json::json!({"id":"old","title":"Old list"})).unwrap();
        let _ = lists.update(Message::Editor(editor::Message::Saved(
            old,
            Ok(Some(result)),
        )));
        assert!(lists.lists.is_empty());
        assert!(lists.editor.is_some());
    }
}
