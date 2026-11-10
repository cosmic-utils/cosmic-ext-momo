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
    filter: String,
    menu: Option<String>,
    loaded: bool,
    error: Option<String>,
    request: u64,
    selected: Option<Timeline>,
    selection_serial: u64,
}

#[derive(Debug, Clone)]
pub enum Message {
    SetClient(Client),
    Manage(Option<String>),
    RequestDelete(String),
    Menu(Option<String>),
    Editor(editor::Message),
    Retry,
    FilterChanged(String),
    Failed(u64, String),
    SetLists(u64, Vec<List>),
    Select(String),
    Deselect,
    TimelineEvent(u64, timeline::Message),
    Timeline(timeline::Message),
}

impl Lists {
    pub fn new(mastodon: Client) -> Self {
        Self {
            editor: None,
            editor_serial: 0,
            mastodon,
            lists: Vec::new(),
            filter: String::new(),
            menu: None,
            loaded: false,
            error: None,
            request: 0,
            selected: None,
            selection_serial: 0,
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.mastodon.is_authenticated()
    }

    pub fn is_editing(&self) -> bool {
        self.editor.is_some()
    }

    pub fn has_menu(&self) -> bool {
        self.menu.is_some()
    }

    pub fn dialog<'a>(&'a self, cache: &'a Cache) -> Option<Element<'a, app::Message>> {
        self.editor.as_ref().map(|editor| {
            if editor.id.is_none() {
                editor.creation_dialog()
            } else {
                editor.view(cache)
            }
        })
    }

    pub fn view<'a>(&'a self, cache: &'a Cache) -> Element<'a, Message> {
        let spacing = cosmic::theme::active().cosmic().spacing;
        if let Some(timeline) = &self.selected {
            let serial = self.selection_serial;
            return widget::column![
                widget::button::text(crate::fl!("reading-back"))
                    .leading_icon(widget::icon::from_name("go-previous-symbolic"))
                    .on_press(Message::Deselect),
                timeline
                    .view(cache)
                    .map(move |message| Message::TimelineEvent(serial, message)),
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
        let header = widget::row![
            widget::text::title2(crate::fl!("lists")),
            widget::space::horizontal(),
            crate::ui::refresh_action(
                !self.loaded,
                crate::fl!("feed-refresh"),
                self.loaded.then_some(Message::Retry)
            ),
            crate::ui::icon_action(
                "list-add-symbolic",
                crate::fl!("list-create"),
                Some(Message::Manage(None))
            ),
        ]
        .spacing(8)
        .align_y(cosmic::iced::Alignment::Center);
        let mut body = widget::column![header]
            .spacing(crate::ui::GAP)
            .width(Length::Fill);
        if let Some(error) = &self.error {
            body = body.push(
                widget::column![
                    widget::text(error),
                    widget::button::standard(crate::fl!("reading-retry")).on_press(Message::Retry)
                ]
                .spacing(8),
            );
        }
        body = body.push(
            widget::text_input(crate::fl!("lists-filter"), &self.filter)
                .on_input(Message::FilterChanged),
        );
        let filter = self.filter.to_lowercase();
        if self.loaded && self.error.is_none() && self.lists.is_empty() {
            body = body.push(crate::ui::empty(
                "view-list-symbolic",
                crate::fl!("lists-empty-title"),
                crate::fl!("lists-empty-description"),
            ));
        } else if !self.lists.is_empty() {
            let rows = self
                .lists
                .iter()
                .filter(|list| filter.is_empty() || list.title.to_lowercase().contains(&filter))
                .fold(
                    widget::settings::section::with_column(
                        widget::list_column().list_item_padding(0),
                    )
                    .title(crate::fl!("lists-yours")),
                    |section, list| {
                        section.add(
                            // Keep the popover outside a Button: libcosmic's Button
                            // overlay adds its absolute position again to nested menus.
                            // The mouse area covers padding; the text button provides
                            // keyboard access, and the menu consumes its own clicks.
                            widget::mouse_area(
                                widget::container(widget::settings::item_row(vec![
                                    widget::text(list.title.clone()).into(),
                                    widget::space::horizontal().into(),
                                    self.list_menu(list),
                                ]))
                                .padding([8, 16])
                                .width(Length::Fill),
                            )
                            .interaction(cosmic::iced::mouse::Interaction::Pointer)
                            .on_press(Message::Select(list.id.clone())),
                        )
                    },
                );
            body = body.push(
                widget::scrollable(rows)
                    .height(Length::Fill)
                    .width(Length::Fill),
            );
        }
        body.spacing(spacing.space_xs)
            .apply(widget::container)
            .max_width(700)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Start)
            .into()
    }

    fn list_menu(&self, list: &List) -> Element<'static, Message> {
        let open = self.menu.as_ref() == Some(&list.id);
        let root = widget::button::icon(widget::icon::from_name("view-more-symbolic"))
            .tooltip(crate::fl!("feed-more"))
            .class(cosmic::style::Button::MenuRoot)
            .on_press(Message::Menu((!open).then(|| list.id.clone())));
        #[cfg(target_os = "linux")]
        let root = root.name(crate::fl!("feed-more"));
        let mut popover = widget::popover(root)
            .position(widget::popover::Position::Bottom)
            .on_close(Message::Menu(None));
        if open {
            let item = |icon: &str, label: String, message| {
                widget::menu::menu_button(vec![
                    widget::icon::from_name(icon.to_owned()).size(16).into(),
                    widget::space::horizontal().width(8).into(),
                    widget::text(label).into(),
                ])
                .on_press(message)
            };
            let content = widget::column![
                item(
                    "edit-symbolic",
                    crate::fl!("list-edit-title"),
                    Message::Manage(Some(list.id.clone()))
                ),
                item(
                    "user-trash-symbolic",
                    crate::fl!("list-delete"),
                    Message::RequestDelete(list.id.clone())
                ),
            ]
            .width(200);
            popover = popover.popup(widget::container(content).class(
                cosmic::style::Container::custom(|theme| {
                    use cosmic::theme::menu_bar::{MenuBarStyle, StyleSheet};
                    let style = theme.appearance(&MenuBarStyle::Default, true);
                    widget::container::Style {
                        background: Some(style.background.into()),
                        border: cosmic::iced::Border {
                            color: style.border_color,
                            width: style.border_width,
                            radius: style.menu_border_radius.into(),
                        },
                        ..Default::default()
                    }
                }),
            ));
        }
        popover.into()
    }

    fn open_editor(&mut self, id: Option<String>, delete: bool) -> Task<app::Message> {
        self.menu = None;
        self.request += 1;
        self.loaded = true;
        self.editor_serial += 1;
        let title = id
            .as_ref()
            .and_then(|id| self.lists.iter().find(|list| &list.id == id))
            .map(|list| list.title.clone())
            .unwrap_or_default();
        let mut editor = editor::Editor::new(id, title, self.editor_serial);
        let creating = editor.id.is_none();
        let task = if delete {
            editor.update(editor::Message::ConfirmDelete, self.mastodon.clone())
        } else {
            editor.load(self.mastodon.clone(), false)
        };
        self.editor_serial = editor.serial;
        self.editor = Some(editor);
        if creating {
            Task::batch([task, widget::text_input::focus(editor::create_name_id())])
        } else {
            task
        }
    }

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        match message {
            Message::FilterChanged(filter) => {
                self.filter = filter;
                self.menu = None;
            }
            Message::Menu(menu) => self.menu = menu,
            Message::Manage(id) => return self.open_editor(id, false),
            Message::RequestDelete(id) => return self.open_editor(Some(id), true),
            Message::Editor(editor::Message::Close)
                if self.editor.as_ref().is_some_and(|editor| !editor.busy()) =>
            {
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
                self.request += 1;
                self.loaded = false;
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
            Message::Deselect => {
                self.selection_serial += 1;
                self.selected = None;
            }
            Message::TimelineEvent(serial, message) if serial == self.selection_serial => {
                return self.update(Message::Timeline(message))
            }
            Message::TimelineEvent(..) => (),
            Message::Select(id) => {
                self.menu = None;
                self.selection_serial += 1;
                let title = self
                    .lists
                    .iter()
                    .find(|list| list.id == id)
                    .map(|list| list.title.clone())
                    .unwrap_or_else(|| crate::fl!("lists"));
                let mut timeline =
                    Timeline::new(self.mastodon.clone(), TimelineKind::List(id)).with_title(title);
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
            subscriptions.push(
                timeline
                    .subscription()
                    .with(self.selection_serial)
                    .map(|(serial, message)| Message::TimelineEvent(serial, message)),
            );
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
