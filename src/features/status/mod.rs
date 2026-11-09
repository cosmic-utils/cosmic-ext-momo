use cosmic::{
    app::Task,
    iced::widget::scrollable::{Direction, Scrollbar},
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
    Reply(String, String),
    Favorite(String, bool),
    Boost(String, bool),
    Bookmark(String, bool),
    OpenLink(String),
    /// Show an image attachment in an overlay: (cached preview URL, original URL).
    ViewImage(String, String),
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

    pub fn none() -> StatusOptions {
        StatusOptions::new(false, false, false, false)
    }
}

pub fn status<'a>(
    status: &'a Status,
    options: StatusOptions,
    cache: &'a Cache,
) -> Element<'a, Message> {
    let spacing = cosmic::theme::active().cosmic().spacing;
    let status = cache.statuses.get(&status.id).unwrap_or(status);
    let reblog_button = reblog_button(cache, status);
    let status = status
        .reblog
        .as_ref()
        .map(|reblog| cache.statuses.get(&reblog.id.to_string()).unwrap_or(reblog))
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
            return post.padding(spacing.space_xs).into();
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
            widget::button::standard(fl!("reading-conversation"))
                .on_press(Message::ExpandStatus(status.id.clone())),
        );
    }
    post.padding(spacing.space_xs).width(Length::Fill).into()
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
        Message::ToggleWarning(_)
        | Message::ToggleMedia(_)
        | Message::ToggleFilter(_, _)
        | Message::ReloadFilters
        | Message::InlineLink(_, _)
        | Message::Copy(_)
        | Message::PollChoice(_, _)
        | Message::Vote(_, _)
        | Message::RefreshPoll(_)
        | Message::Report(_, _) => cosmic::task::message(app::Message::Status(message)),
        Message::OpenAccount(account) => cosmic::task::message(app::Message::ToggleContextPage(
            app::ContextPage::Account(account),
        )),
        Message::ExpandStatus(id) => cosmic::task::message(app::Message::ToggleContextPage(
            app::ContextPage::Status(id),
        )),
        Message::Reply(status_id, username) => {
            let state = compose::State::reply(status_id, username);
            cosmic::task::message(app::Message::Dialog(app::DialogAction::Open(
                app::Dialog::Compose(state),
            )))
        }
        Message::Favorite(status_id, favorited) => cosmic::task::message(app::Message::Status(
            Message::Favorite(status_id, favorited),
        )),
        Message::Boost(status_id, boosted) => {
            cosmic::task::message(app::Message::Status(Message::Boost(status_id, boosted)))
        }
        Message::Bookmark(status_id, bookmarked) => cosmic::task::message(app::Message::Status(
            Message::Bookmark(status_id, bookmarked),
        )),
        Message::OpenLink(url) => cosmic::task::message(app::Message::Open(url.to_string())),
        Message::ViewImage(preview_url, original_url) => {
            cosmic::task::message(app::Message::Dialog(app::DialogAction::Open(
                app::Dialog::Image(preview_url, original_url),
            )))
        }
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
    let spacing = cosmic::theme::active().cosmic().spacing;

    let delete_button = cache.is_me(&status.account.id).then(|| {
        widget::button::icon(widget::icon::from_name("user-trash-symbolic"))
            .on_press(Message::Delete(status.id.clone()))
    });

    if !options.actions {
        return None;
    }
    let action = |name: &str, count: u32, selected: bool| {
        widget::button::icon(widget::icon::from_name(name.to_string()))
            .label(count.to_string())
            .class(if selected {
                cosmic::theme::Button::Suggested
            } else {
                cosmic::theme::Button::Icon
            })
    };
    let buttons = widget::row![
        action("mail-replied-symbolic", status.replies_count, false).on_press_maybe(
            cache.me.is_some().then_some(Message::Reply(
                status.id.clone(),
                status.account.acct.clone()
            ))
        ),
        action(
            "emblem-shared-symbolic",
            status.reblogs_count,
            status.reblogged.unwrap_or(false)
        )
        .on_press_maybe(
            status
                .reblogged
                .map(|value| Message::Boost(status.id.clone(), value))
        ),
        action(
            "starred-symbolic",
            status.favourites_count,
            status.favourited.unwrap_or(false)
        )
        .on_press_maybe(
            status
                .favourited
                .map(|value| Message::Favorite(status.id.clone(), value))
        ),
        widget::button::icon(widget::icon::from_name("bookmark-new-symbolic"))
            .class(if status.bookmarked.unwrap_or(false) {
                cosmic::theme::Button::Suggested
            } else {
                cosmic::theme::Button::Icon
            })
            .on_press_maybe(
                status
                    .bookmarked
                    .map(|value| Message::Bookmark(status.id.clone(), value))
            ),
    ]
    .spacing(spacing.space_xs);
    let secondary =
        widget::row![
            widget::button::standard(fl!("reading-copy-text")).on_press(Message::Copy(format!(
                "{}\n{}",
                status.spoiler_text,
                html::plain(&status.content)
            )))
        ]
        .push_maybe(
            (cache.me.is_some() && !cache.is_me(&status.account.id)).then(|| {
                widget::button::standard(fl!("reading-report")).on_press(Message::Report(
                    status.account.clone(),
                    Some(status.id.clone()),
                ))
            }),
        )
        .push_maybe(delete_button)
        .spacing(spacing.space_xs);
    Some(
        widget::column![buttons, secondary]
            .spacing(spacing.space_xs)
            .into(),
    )
}

fn media<'a>(
    status: &'a Status,
    cache: &'a Cache,
    options: StatusOptions,
    density: FeedDensity,
) -> Option<cosmic::iced::widget::Scrollable<'a, Message, cosmic::Theme>> {
    if density == FeedDensity::TextOnly {
        return None;
    }
    let compact = density == FeedDensity::Compact;

    let spacing = cosmic::theme::active().cosmic().spacing;

    let attachments = status
        .media_attachments
        .iter()
        .map(|media| {
            let is_picture = matches!(
                media.r#type,
                megalodon::entities::attachment::AttachmentType::Image
                    | megalodon::entities::attachment::AttachmentType::Gifv
            );
            let on_press = match &media.preview_url {
                Some(preview_url) if is_picture => {
                    Message::ViewImage(preview_url.clone(), media.url.clone())
                }
                _ => Message::OpenLink(media.url.clone()),
            };

            let button = widget::button::image(
                media
                    .preview_url
                    .as_ref()
                    .and_then(|url| cache.handles.get(url))
                    .cloned()
                    .unwrap_or(crate::cache::fallback_handle()),
            )
            .on_press(on_press);
            let button = if compact {
                button.width(80.0).height(80.0)
            } else {
                button.width(240.0)
            };
            widget::column![
                button,
                widget::selectable_text(
                    media
                        .description
                        .clone()
                        .unwrap_or_else(|| fl!("reading-no-description"))
                )
            ]
            .width(240.0)
            .into()
        })
        .collect::<Vec<Element<Message>>>();

    let media = (!status.media_attachments.is_empty() && options.media).then_some({
        widget::scrollable(widget::row(attachments).spacing(spacing.space_xxs))
            .direction(Direction::Horizontal(Scrollbar::new()))
    });
    media
}

fn tags(status: &Status, options: StatusOptions) -> Option<Element<'_, Message>> {
    let spacing = cosmic::theme::active().cosmic().spacing;

    let tags: Option<Element<_>> = (!status.tags.is_empty() && options.tags).then(|| {
        let tags = status
            .tags
            .iter()
            .map(|tag| {
                widget::button::suggested(format!("#{}", tag.name.clone()))
                    .on_press(Message::OpenLink(tag.url.clone()))
                    .into()
            })
            .collect::<Vec<Element<Message>>>();
        widget::flex_row(tags).spacing(spacing.space_xxs).into()
    });
    tags
}

fn header<'a>(
    status: &'a Status,
    cache: &'a Cache,
    density: FeedDensity,
) -> cosmic::widget::Row<'a, Message, cosmic::Theme> {
    let spacing = cosmic::theme::active().cosmic().spacing;
    let avatar_size = if density == FeedDensity::Full {
        50.0
    } else {
        36.0
    };

    let header = widget::row![
        widget::button::image(
            cache
                .handles
                .get(&status.account.avatar)
                .cloned()
                .unwrap_or(crate::cache::fallback_handle()),
        )
        .width(avatar_size)
        .height(avatar_size)
        .on_press(Message::OpenAccount(status.account.clone())),
        widget::column![]
            .push_maybe(
                (!status.account.display_name.is_empty())
                    .then(|| { widget::text(status.account.display_name.clone()).size(18) })
            )
            .push(
                widget::button::link(html::full_acct(&status.account))
                    .on_press(Message::OpenAccount(status.account.clone())),
            )
            .push(widget::text::caption(
                status
                    .created_at
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M %Z")
                    .to_string()
            ))
            .push_maybe(status.edited_at.map(|date| {
                widget::text::caption(format!(
                    "{} {}",
                    fl!("reading-edited"),
                    date.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M %Z")
                ))
            }))
            .align_x(Alignment::Start)
            .spacing(spacing.space_xs),
    ]
    .align_y(Alignment::Center)
    .spacing(spacing.space_xs);
    header
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
                widget::text(format!("{} boosted", status.account.display_name)),
            ]
            .spacing(spacing.space_xs),
        )
        .on_press(Message::OpenAccount(status.account.clone())),
    )
}
