pub mod boost;
pub mod favorite;

use cosmic::{
    app::Task,
    iced::{Alignment, Length},
    widget, Apply, Element,
};
use megalodon::entities::{Account, Status};

use crate::{
    app,
    cache::{self, Cache},
    config::FeedDensity,
    features::{
        compose,
        reading::{
            self,
            filters::{Context, Decision},
            html,
        },
    },
    fl,
};

#[derive(Debug, Clone)]
pub enum Message {
    Menu(Option<String>),
    MenuAction(Box<Message>),
    ToggleWarning(String),
    ToggleMedia(String),
    ToggleFilter(String, Context),
    ReloadFilters,
    InlineLink(String, String),
    Copy(String),
    PollChoice(String, u32),
    Vote(String, String),
    RefreshPoll(String),
    Report(Account, Option<String>),
    OpenAccount(Account),
    ExpandStatus(String),
    Reply(String),
    Edit(String),
    Favorite(String, bool),
    Boost(String, bool),
    Bookmark(String, bool),
    OpenLink(String),
    /// Open the post media gallery at the selected index.
    ViewMedia(Vec<crate::media::Attachment>, usize, String),
    /// Request to delete one of the authenticated user's own statuses;
    /// opens a confirmation dialog rather than deleting immediately.
    Delete(String),
}

#[derive(Debug, Copy, Clone)]
pub struct StatusOptions {
    media: bool,
    tags: bool,
    actions: bool,
    expand: bool,
    context: Context,
}

impl StatusOptions {
    pub fn new(media: bool, tags: bool, actions: bool, expand: bool) -> Self {
        Self {
            media,
            tags,
            actions,
            expand,
            context: Context::Home,
        }
    }

    pub fn context(mut self, context: Context) -> Self {
        self.context = context;
        self
    }

    pub fn all() -> StatusOptions {
        StatusOptions::new(true, true, true, true)
    }
}

pub fn status<'a>(
    status: &'a Status,
    options: StatusOptions,
    cache: &'a Cache,
) -> Element<'a, Message> {
    let spacing = cosmic::theme::active().cosmic().spacing;
    if cache.deleted.contains(&status.id)
        || status
            .reblog
            .as_ref()
            .is_some_and(|post| cache.deleted.contains(&post.id))
    {
        return widget::column![].into();
    }
    let status = cache.statuses.get(&status.id).unwrap_or(status);
    let reblog_button = reblog_button(cache, status);
    let status = status
        .reblog
        .as_ref()
        .map(|reblog| cache.statuses.get(&reblog.id).unwrap_or(reblog))
        .unwrap_or(status);

    let density = cache.feed_density;

    let decision = cache.reading.filters.decision(status, options.context);
    match &decision {
        Decision::Hide => return widget::space::vertical().height(0).into(),
        Decision::Loading => return widget::text(fl!("reading-filters-loading")).into(),
        Decision::Failed => {
            return widget::column![
                widget::text(fl!("reading-filters-error")),
                widget::button::standard(fl!("reading-retry")).on_press(Message::ReloadFilters)
            ]
            .into()
        }
        Decision::Warn(titles, _)
            if !cache
                .reading
                .filter_reveals
                .contains(&(status.id.clone(), options.context)) =>
        {
            return widget::column![
                widget::text(format!(
                    "{}: {}",
                    fl!("reading-filtered"),
                    titles.join(", ")
                )),
                widget::button::standard(fl!("reading-show-post"))
                    .on_press(Message::ToggleFilter(status.id.clone(), options.context))
            ]
            .into();
        }
        _ => (),
    }
    let mut post =
        widget::column![reblog_button, header(status, cache, density)].spacing(spacing.space_xs);
    if matches!(decision, Decision::Warn(..)) {
        post = post.push(
            widget::button::standard(fl!("reading-hide-post"))
                .on_press(Message::ToggleFilter(status.id.clone(), options.context)),
        );
    }
    if !status.spoiler_text.is_empty() {
        post = post
            .push(widget::selectable_text(status.spoiler_text.clone()))
            .push(
                widget::button::standard(if cache.reading.warnings.contains(&status.id) {
                    fl!("reading-hide-post")
                } else {
                    fl!("reading-show-post")
                })
                .on_press(Message::ToggleWarning(status.id.clone())),
            );
        if !cache.reading.content_visible(status) {
            return post
                .padding(if density == FeedDensity::Full { 16 } else { 12 })
                .into();
        }
    }
    post = post.push(content(status));
    if let Some(poll) = status.poll.as_ref() {
        post = post.push(poll_view(status, poll, cache));
    }
    let blur = matches!(&decision, Decision::Blur(_))
        || matches!(&decision, Decision::Warn(_, media_titles) if !media_titles.is_empty());
    let hidden_media = !cache.reading.media_visible(status, blur);
    if (status.sensitive || blur) && (!status.media_attachments.is_empty() || status.card.is_some())
    {
        let title = match &decision {
            Decision::Blur(titles) | Decision::Warn(_, titles) if !titles.is_empty() => {
                format!("{}: {}", fl!("reading-filtered-media"), titles.join(", "))
            }
            _ => fl!("reading-sensitive-media"),
        };
        post = post.push(widget::text(title)).push(
            widget::button::standard(if hidden_media {
                fl!("reading-show-media")
            } else {
                fl!("reading-hide-media")
            })
            .on_press(Message::ToggleMedia(status.id.clone())),
        );
    }
    if !hidden_media {
        post = post
            .push_maybe(card(status, cache, density))
            .push_maybe(media(status, cache, options, density));
    }
    post = post
        .push_maybe(tags(status, options))
        .push_maybe(actions(status, options, cache));
    if options.expand {
        post = post.push(
            widget::button::link(fl!("reading-conversation"))
                .on_press(Message::ExpandStatus(status.id.clone())),
        );
    }
    post.padding(if density == FeedDensity::Full { 16 } else { 12 })
        .width(Length::Fill)
        .into()
}

fn card<'a>(
    status: &'a Status,
    cache: &'a Cache,
    density: FeedDensity,
) -> Option<Element<'a, Message>> {
    if density == FeedDensity::TextOnly {
        return None;
    }
    let compact = density == FeedDensity::Compact;

    let spacing = cosmic::theme::active().cosmic().spacing;
    status.card.as_ref().map(|card| {
        let avatar = card.image.as_ref().map(|image| {
            let fallback = cache::fallback_avatar();
            let handle = cache
                .handles
                .get(image)
                .map(widget::image)
                .unwrap_or(fallback);
            if compact {
                handle
                    .width(150.0)
                    .height(Length::Fill)
                    .content_fit(cosmic::iced::ContentFit::Cover)
            } else {
                handle
            }
        });

        let text = widget::column![
            widget::text::title4(&card.title).wrapping(cosmic::iced::core::text::Wrapping::Word),
            widget::text(&card.description).wrapping(cosmic::iced::core::text::Wrapping::Word),
        ]
        .spacing(spacing.space_xs)
        .padding(spacing.space_xs)
        .width(Length::Fill);

        let content: Element<'_, Message> = if compact {
            widget::row![avatar, text]
                .spacing(spacing.space_xs)
                .align_y(Alignment::Center)
                .width(Length::Fill)
                .into()
        } else {
            widget::column![avatar, text].into()
        };

        content
            .apply(widget::container)
            .width(Length::Fill)
            .class(cosmic::style::Container::Dialog(false))
            .apply(widget::button::custom)
            .width(Length::Fill)
            .class(cosmic::style::Button::Image)
            .on_press(Message::OpenLink(card.url.clone()))
            .into()
    })
}

pub fn update(message: Message) -> Task<app::Message> {
    match message {
        Message::Menu(_) | Message::MenuAction(_) => {
            cosmic::task::message(app::Message::Status(message))
        }
        Message::ToggleWarning(_)
        | Message::ToggleMedia(_)
        | Message::ToggleFilter(_, _)
        | Message::ReloadFilters
        | Message::InlineLink(_, _)
        | Message::Copy(_)
        | Message::PollChoice(_, _)
        | Message::Vote(_, _)
        | Message::RefreshPoll(_)
        | Message::Report(_, _)
        | Message::Favorite(_, _)
        | Message::Boost(_, _)
        | Message::Bookmark(_, _) => cosmic::task::message(app::Message::Status(message)),
        Message::OpenAccount(account) => cosmic::task::message(app::Message::ToggleContextPage(
            app::ContextPage::Account(account),
        )),
        Message::ExpandStatus(id) => cosmic::task::message(app::Message::ToggleContextPage(
            app::ContextPage::Status(id),
        )),
        Message::Reply(status_id) => {
            cosmic::task::message(app::Message::Compose(compose::Message::Reply(status_id)))
        }
        Message::Edit(id) => {
            cosmic::task::message(app::Message::Compose(compose::Message::Edit(id)))
        }
        Message::OpenLink(url) => cosmic::task::message(app::Message::Open(url)),
        Message::ViewMedia(images, index, id) => cosmic::task::message(app::Message::Dialog(
            app::DialogAction::Open(app::Dialog::Media(images, index, id)),
        )),
        Message::Delete(status_id) => cosmic::task::message(app::Message::Dialog(
            app::DialogAction::Open(app::Dialog::DeleteStatus(status_id)),
        )),
    }
}

fn actions<'a>(
    status: &'a Status,
    options: StatusOptions,
    cache: &'a Cache,
) -> Option<Element<'a, Message>> {
    if !options.actions {
        return None;
    }
    let action = |icon: &str,
                  label: String,
                  count: Option<u32>,
                  selected: bool,
                  message: Option<Message>| {
        let label = count
            .map(|count| format!("{label} ({count})"))
            .unwrap_or(label);
        let mut button = widget::button::icon(widget::icon::from_name(icon.to_string()))
            .icon_size(18)
            .padding(8)
            .tooltip(label.clone())
            .class(if selected {
                cosmic::theme::Button::Suggested
            } else {
                cosmic::theme::Button::Icon
            })
            .on_press_maybe(message);
        if let Some(count) = count.filter(|count| *count > 0) {
            let count = if count >= 1_000_000_000 {
                format!("{:.1}b", count as f64 / 1_000_000_000.0)
            } else if count >= 1_000_000 {
                format!("{:.1}m", count as f64 / 1_000_000.0)
            } else if count >= 1_000 {
                format!("{:.1}k", count as f64 / 1_000.0)
            } else {
                count.to_string()
            };
            button = button.label(count);
        }
        #[cfg(target_os = "linux")]
        let button = button.name(label);
        button
    };
    let authenticated = cache.me.is_some();
    let boosted = status.reblogged.unwrap_or(false);
    let favorited = status.favourited.unwrap_or(false);
    let bookmarked = status.bookmarked.unwrap_or(false);
    let public = matches!(
        status.visibility,
        megalodon::entities::status::StatusVisibility::Public
            | megalodon::entities::status::StatusVisibility::Unlisted
    );
    let buttons = widget::row![
        action(
            "mail-replied-symbolic",
            fl!("reply"),
            Some(status.replies_count),
            false,
            authenticated.then(|| Message::Reply(status.id.clone()))
        ),
        action(
            "emblem-shared-symbolic",
            if boosted {
                fl!("feed-unboost")
            } else {
                fl!("feed-boost")
            },
            Some(status.reblogs_count),
            boosted,
            (authenticated && public).then(|| Message::Boost(status.id.clone(), boosted))
        ),
        action(
            "starred-symbolic",
            if favorited {
                fl!("feed-unfavorite")
            } else {
                fl!("feed-favorite")
            },
            Some(status.favourites_count),
            favorited,
            authenticated.then(|| Message::Favorite(status.id.clone(), favorited))
        ),
        action(
            "bookmark-new-symbolic",
            if bookmarked {
                fl!("feed-unbookmark")
            } else {
                fl!("feed-bookmark")
            },
            None,
            bookmarked,
            authenticated.then(|| Message::Bookmark(status.id.clone(), bookmarked))
        ),
        widget::space::horizontal(),
    ]
    .push_maybe(cache.is_me(&status.account.id).then(|| {
        action(
            "edit-symbolic",
            fl!("compose-edit"),
            None,
            false,
            Some(Message::Edit(status.id.clone())),
        )
    }))
    .push(more_menu(status, cache))
    .spacing(4)
    .align_y(Alignment::Center);
    Some(buttons.into())
}

fn more_menu(status: &Status, cache: &Cache) -> Element<'static, Message> {
    let item = |icon: &str, label: String, message| {
        widget::menu::menu_button(vec![
            widget::icon::from_name(icon.to_owned()).size(16).into(),
            widget::space::horizontal().width(8).into(),
            widget::text(label).into(),
        ])
        .on_press(Message::MenuAction(Box::new(message)))
    };
    let open = cache.reading.post_menu.as_deref() == Some(status.id.as_str());
    let root = widget::button::icon(widget::icon::from_name("view-more-symbolic"))
        .tooltip(fl!("feed-more"))
        .on_press(Message::Menu((!open).then(|| status.id.clone())))
        .class(cosmic::style::Button::MenuRoot);
    #[cfg(target_os = "linux")]
    let root = root.name(fl!("feed-more"));
    let mut popover = widget::popover(root)
        .position(widget::popover::Position::Bottom)
        .on_close(Message::Menu(None));
    if open {
        let mut items = widget::column![item(
            "edit-copy-symbolic",
            fl!("reading-copy-text"),
            Message::Copy(format!(
                "{}\n{}",
                status.spoiler_text,
                html::plain(&status.content)
            )),
        )]
        .spacing(0)
        .width(220);
        if cache.me.is_some() && !cache.is_me(&status.account.id) {
            items = items.push(item(
                "dialog-warning-symbolic",
                fl!("reading-report"),
                Message::Report(status.account.clone(), Some(status.id.clone())),
            ));
        }
        if cache.is_me(&status.account.id) {
            items = items
                .push(item(
                    "edit-symbolic",
                    fl!("compose-edit"),
                    Message::Edit(status.id.clone()),
                ))
                .push(item(
                    "user-trash-symbolic",
                    fl!("delete-status"),
                    Message::Delete(status.id.clone()),
                ));
        }
        popover = popover.popup(
            widget::container(items).class(cosmic::style::Container::custom(|theme| {
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
            })),
        );
    }
    popover.into()
}

fn media<'a>(
    status: &'a Status,
    cache: &'a Cache,
    options: StatusOptions,
    density: FeedDensity,
) -> Option<Element<'a, Message>> {
    if density == FeedDensity::TextOnly {
        return None;
    }
    let compact = density == FeedDensity::Compact;
    let single = status.media_attachments.len() == 1;
    let gallery_height = if compact { 120.0 } else { 240.0 };

    let spacing = cosmic::theme::active().cosmic().spacing;

    let images: Vec<_> = status
        .media_attachments
        .iter()
        .filter_map(crate::media::Attachment::from_media)
        .collect();

    let attachments = status
        .media_attachments
        .iter()
        .map(|media| {
            let on_press =
                if let Some(index) = images.iter().position(|image| image.url == media.url) {
                    Message::ViewMedia(images.clone(), index, status.id.clone())
                } else {
                    Message::OpenLink(media.url.clone())
                };

            let width = Length::Fill;
            let height = if single {
                Length::Shrink
            } else {
                Length::Fixed(gallery_height)
            };
            let preview = widget::image(
                media
                    .preview_url
                    .as_ref()
                    .and_then(|url| cache.handles.get(url))
                    .cloned()
                    .unwrap_or_else(crate::cache::fallback_handle),
            )
            .width(width)
            .height(height)
            .expand(single)
            .content_fit(if single {
                cosmic::iced::ContentFit::Contain
            } else {
                cosmic::iced::ContentFit::Cover
            })
            .border_radius([9.0; 4]);
            let preview = widget::container(preview)
                .width(width)
                .max_height(if single { 550.0 } else { gallery_height });
            let button = widget::button::custom(preview)
                .class(cosmic::style::Button::Image)
                .padding(0)
                .width(width)
                .height(height)
                .on_press(on_press.clone());
            #[cfg(target_os = "linux")]
            let button = button.name(
                media
                    .description
                    .clone()
                    .filter(|text| !text.is_empty())
                    .unwrap_or_else(|| fl!("media-open-attachment")),
            );
            let is_video = matches!(
                media.r#type,
                megalodon::entities::attachment::AttachmentType::Video
                    | megalodon::entities::attachment::AttachmentType::Gifv
            );
            if is_video {
                cosmic::iced::widget::stack![
                    button,
                    widget::container(
                        widget::button::icon(widget::icon::from_name(
                            "media-playback-start-symbolic"
                        ))
                        .icon_size(32)
                        .class(cosmic::style::Button::Standard)
                        .tooltip(fl!("media-play"))
                        .on_press(on_press),
                    )
                    .center(Length::Fill),
                ]
                .into()
            } else {
                button.into()
            }
        })
        .collect::<Vec<Element<Message>>>();

    if status.media_attachments.is_empty() || !options.media {
        return None;
    }
    if single {
        attachments.into_iter().next()
    } else {
        let mut grid = widget::column![]
            .spacing(spacing.space_xxs)
            .width(Length::Fill);
        let mut attachments = attachments.into_iter();
        while let Some(first) = attachments.next() {
            let mut row = widget::row![first]
                .spacing(spacing.space_xxs)
                .width(Length::Fill);
            if let Some(second) = attachments.next() {
                row = row.push(second);
            }
            grid = grid.push(row);
        }
        Some(grid.into())
    }
}

fn tags(status: &Status, options: StatusOptions) -> Option<Element<'_, Message>> {
    let spacing = cosmic::theme::active().cosmic().spacing;

    // Tags already written in the post body are shown there; only list the rest.
    let body = html::plain(&status.content).to_lowercase();
    let hidden = |name: &str| {
        let needle = format!("#{}", name.to_lowercase());
        body.match_indices(&needle).any(|(i, _)| {
            !body[i + needle.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
    };
    let extra: Vec<_> = status
        .tags
        .iter()
        .filter(|tag| !hidden(&tag.name))
        .collect();
    (!extra.is_empty() && options.tags).then(|| {
        let tags = extra
            .into_iter()
            .map(|tag| {
                widget::button::link(format!("#{}", tag.name))
                    .on_press(Message::OpenLink(tag.url.clone()))
                    .into()
            })
            .collect::<Vec<Element<Message>>>();
        widget::flex_row(tags).spacing(spacing.space_xxs).into()
    })
}

fn header<'a>(
    status: &'a Status,
    cache: &'a Cache,
    density: FeedDensity,
) -> cosmic::widget::Row<'a, Message, cosmic::Theme> {
    let avatar_size = if density == FeedDensity::Full {
        40.0
    } else {
        32.0
    };
    let display_name = if status.account.display_name.trim().is_empty() {
        status.account.username.clone()
    } else {
        status.account.display_name.clone()
    };
    let identity = widget::column![
        widget::button::link(display_name)
            .padding(0)
            .on_press(Message::OpenAccount(status.account.clone())),
        widget::text::caption(html::full_acct(&status.account)),
    ]
    .spacing(0)
    .width(Length::Fill);
    let date = status.created_at.with_timezone(&chrono::Local);
    let label = if date.date_naive() == chrono::Local::now().date_naive() {
        date.format("%H:%M").to_string()
    } else {
        date.format("%b %-d").to_string()
    };
    let mut exact = date.format("%Y-%m-%d %H:%M %Z").to_string();
    if let Some(edited) = status.edited_at {
        exact.push_str(&format!(
            "\n{} {}",
            fl!("reading-edited"),
            edited
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M %Z")
        ));
    }
    let time = widget::tooltip(
        widget::button::link(if status.edited_at.is_some() {
            format!("{label} · {}", fl!("reading-edited"))
        } else {
            label
        })
        .on_press(Message::ExpandStatus(status.id.clone())),
        widget::text(exact),
        widget::tooltip::Position::Top,
    );
    let avatar = widget::button::image(
        cache
            .handles
            .get(&status.account.avatar)
            .cloned()
            .unwrap_or_else(crate::cache::fallback_handle),
    )
    .width(avatar_size)
    .height(avatar_size)
    .on_press(Message::OpenAccount(status.account.clone()));
    #[cfg(target_os = "linux")]
    let avatar = avatar.name(format!(
        "{}: {}",
        fl!("media-view-profile"),
        status.account.acct
    ));
    widget::row![
        avatar,
        identity,
        widget::column![time]
            .push_maybe(match status.visibility {
                megalodon::entities::status::StatusVisibility::Private => Some(widget::tooltip(
                    widget::icon::from_name("system-lock-screen-symbolic").size(14),
                    widget::text(fl!("feed-followers-only")),
                    widget::tooltip::Position::Top
                )),
                megalodon::entities::status::StatusVisibility::Direct => Some(widget::tooltip(
                    widget::icon::from_name("mail-read-symbolic").size(14),
                    widget::text(fl!("feed-mentioned-only")),
                    widget::tooltip::Position::Top
                )),
                _ => None,
            })
            .spacing(4)
            .align_x(Alignment::End),
    ]
    .spacing(8)
    .align_y(Alignment::Start)
}

fn content(status: &Status) -> Element<'_, Message> {
    html::view(&status.content, |url| {
        Message::InlineLink(status.id.clone(), url)
    })
}

fn poll_view<'a>(
    status: &'a Status,
    original: &'a megalodon::entities::Poll,
    cache: &'a Cache,
) -> Element<'a, Message> {
    let poll = cache.reading.polls.get(&original.id).unwrap_or(original);
    let choices = cache
        .reading
        .choices
        .get(&poll.id)
        .cloned()
        .unwrap_or_default();
    let busy = cache.reading.poll_busy.contains(&poll.id);
    let can_vote =
        cache.me.is_some() && reading::poll_open(poll) && !poll.voted.unwrap_or(false) && !busy;
    let mut view = widget::column![].spacing(8);
    for (index, option) in poll.options.iter().enumerate() {
        let label = match option.votes_count {
            Some(votes) => format!("{} — {}", option.title, votes),
            None => option.title.clone(),
        };
        if can_vote {
            let id = poll.id.clone();
            if poll.multiple {
                view = view.push(
                    widget::checkbox(choices.contains(&(index as u32)))
                        .label(label)
                        .on_toggle(move |_| Message::PollChoice(id.clone(), index as u32)),
                );
            } else {
                view = view.push(widget::radio(
                    widget::text(label),
                    index as u32,
                    choices.first().copied(),
                    move |index| Message::PollChoice(id.clone(), index),
                ));
            }
        } else {
            view = view.push(widget::selectable_text(label));
        }
    }
    view = view.push(widget::text::caption(format!(
        "{}: {}",
        fl!("reading-votes"),
        poll.votes_count
    )));
    if let Some(expiry) = poll.expires_at {
        view = view.push(widget::text::caption(format!(
            "{} {}",
            if reading::poll_open(poll) {
                fl!("reading-poll-ends")
            } else {
                fl!("reading-poll-ended")
            },
            expiry
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M %Z")
        )));
    }
    if can_vote {
        view = view.push(
            widget::button::suggested(fl!("reading-vote")).on_press_maybe(
                reading::valid_choices(poll, &choices)
                    .then_some(Message::Vote(status.id.clone(), poll.id.clone())),
            ),
        );
    }
    view = view.push(
        widget::button::standard(fl!("reading-refresh-poll"))
            .on_press_maybe((!busy).then_some(Message::RefreshPoll(poll.id.clone()))),
    );
    if busy {
        view = view.push(widget::indeterminate_circular().size(24.));
    }
    if let Some(error) = cache.reading.poll_errors.get(&poll.id) {
        view = view.push(widget::text(error));
    }
    view.into()
}

fn reblog_button<'a>(cache: &'a Cache, status: &'a Status) -> Option<widget::Button<'a, Message>> {
    let spacing = cosmic::theme::active().cosmic().spacing;

    (status.reblog.is_some()).then_some(
        widget::button::custom(
            widget::row![
                cache
                    .handles
                    .get(&status.account.avatar)
                    .map(|avatar| widget::image(avatar).width(20).height(20))
                    .unwrap_or(crate::cache::fallback_avatar().width(20).height(20)),
                widget::text::caption(format!(
                    "{} · {}",
                    if status.account.display_name.is_empty() {
                        &status.account.acct
                    } else {
                        &status.account.display_name
                    },
                    fl!("feed-boosted")
                )),
            ]
            .spacing(spacing.space_xs),
        )
        .on_press(Message::OpenAccount(status.account.clone())),
    )
}
