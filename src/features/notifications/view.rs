use super::{
    model::{self, Group},
    Message, Notifications,
};
use crate::{
    cache::Cache,
    features::{
        reading::{
            filters::{Context, Decision},
            html,
        },
        status,
    },
    fl, ui,
};
use cosmic::{
    iced::{Alignment, Length},
    widget, Element,
};
use megalodon::entities::notification::NotificationType as Kind;
use unicode_segmentation::UnicodeSegmentation;

pub fn notification<'a>(
    group: &Group,
    inbox: &Notifications,
    cache: &Cache,
) -> Element<'a, Message> {
    let busy = group.ids.iter().any(|id| inbox.pending.contains(id)) || inbox.clearing;
    let mut identity = widget::column![].spacing(2).width(Length::Fill);
    if let Some(actor) = group.actors.first() {
        identity = identity
            .push(
                widget::button::link(model::actor_name(actor))
                    .on_press(Message::Status(status::Message::OpenAccount(actor.clone()))),
            )
            .push(widget::text::caption(event_label(&group.kind)));
    } else {
        identity = identity.push(widget::text::heading(event_label(&group.kind)));
    }
    let mut avatars = widget::row![].spacing(4).align_y(Alignment::Center);
    if let Some(actor) = group.actors.first() {
        avatars = avatars.push(
            widget::button::image(
                cache
                    .handles
                    .get(&actor.avatar)
                    .cloned()
                    .unwrap_or_else(crate::cache::fallback_handle),
            )
            .width(36)
            .height(36)
            .on_press(Message::Status(status::Message::OpenAccount(actor.clone()))),
        );
    }
    let local = group.created_at.with_timezone(&chrono::Local);
    let timestamp = widget::tooltip(
        widget::text::caption(local.format("%H:%M").to_string()),
        widget::text(local.format("%Y-%m-%d %H:%M %Z").to_string()),
        widget::tooltip::Position::Top,
    );
    let mut body = widget::column![widget::row![
        widget::container(widget::icon::from_name(event_icon(&group.kind)).size(18)).padding(6),
        avatars,
        identity,
        timestamp,
    ]
    .push_maybe(
        (group.kind != Kind::FollowRequest || group.actors.is_empty()).then(|| ui::icon_action(
            "window-close-symbolic",
            fl!("activity-dismiss"),
            (!busy).then_some(Message::Dismiss(group.ids.clone()))
        ))
    )
    .spacing(8)
    .align_y(Alignment::Start)]
    .spacing(12);
    if group.actors.len() > 1 {
        body = body.push(
            widget::button::link(format!(
                "+{} · {}",
                group.actors.len() - 1,
                fl!("activity-people")
            ))
            .on_press(Message::TogglePeople(group.key.clone())),
        );
        if inbox.expanded.contains(&group.key) {
            for actor in group.actors.iter().skip(1) {
                body = body.push(
                    widget::row![
                        widget::button::image(
                            cache
                                .handles
                                .get(&actor.avatar)
                                .cloned()
                                .unwrap_or_else(crate::cache::fallback_handle),
                        )
                        .width(36)
                        .height(36)
                        .on_press(Message::Status(status::Message::OpenAccount(actor.clone()))),
                        widget::column![
                            widget::button::link(model::actor_name(actor)).on_press(
                                Message::Status(status::Message::OpenAccount(actor.clone()))
                            ),
                            widget::text::caption(html::full_acct(actor)),
                        ]
                        .spacing(2),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );
            }
        }
    }
    if let Some(post) = &group.post {
        match cache.reading.filters.decision(post, Context::Notifications) {
            Decision::Warn(titles, _)
                if !cache
                    .reading
                    .filter_reveals
                    .contains(&(post.id.clone(), Context::Notifications)) =>
            {
                body = body
                    .push(widget::text(format!(
                        "{}: {}",
                        fl!("reading-filtered"),
                        titles.join(", ")
                    )))
                    .push(
                        widget::button::standard(fl!("activity-show-preview")).on_press(
                            Message::Status(status::Message::ToggleFilter(
                                post.id.clone(),
                                Context::Notifications,
                            )),
                        ),
                    );
            }
            Decision::Hide | Decision::Loading | Decision::Failed => (),
            _ => {
                if !post.spoiler_text.is_empty() && !cache.reading.content_visible(post) {
                    body = body
                        .push(widget::text::caption(fl!("content-warning")))
                        .push(widget::text(post.spoiler_text.clone()))
                        .push(
                            widget::button::standard(fl!("activity-show-preview")).on_press(
                                Message::Status(status::Message::ToggleWarning(post.id.clone())),
                            ),
                        );
                } else {
                    let text = html::plain(&post.content);
                    let mut excerpt = text.graphemes(true).take(220).collect::<String>();
                    if excerpt.len() < text.len() {
                        excerpt.push('…');
                    }
                    let mut preview = widget::column![widget::text(excerpt)]
                        .spacing(6)
                        .width(Length::Fill);
                    if !post.media_attachments.is_empty() {
                        preview = preview.push(widget::text::caption(format!(
                            "{} · {}",
                            post.media_attachments.len(),
                            fl!("activity-attachments")
                        )));
                    }
                    if post.poll.is_some() {
                        preview = preview.push(widget::text::caption(fl!("activity-poll")));
                    }
                    body = body.push(
                        widget::button::custom(
                            widget::container(preview)
                                .padding(12)
                                .width(Length::Fill)
                                .class(cosmic::style::Container::Dialog(false)),
                        )
                        .class(cosmic::style::Button::Image)
                        .width(Length::Fill)
                        .on_press(Message::Status(
                            status::Message::ExpandStatus(post.id.clone()),
                        )),
                    );
                    if group.kind == Kind::Mention && cache.me.is_some() {
                        body = body
                            .push(widget::button::standard(fl!("reply")).on_press(
                                Message::Status(status::Message::Reply(post.id.clone())),
                            ));
                    }
                }
            }
        }
    }
    if group.kind == Kind::FollowRequest {
        if let Some(actor) = group.actors.first() {
            body = body.push(
                widget::row![
                    widget::button::suggested(fl!("activity-accept")).on_press_maybe(
                        (!busy).then_some(Message::FollowDecision(
                            group.ids[0].clone(),
                            actor.id.clone(),
                            true
                        ))
                    ),
                    widget::button::standard(fl!("activity-decline")).on_press_maybe(
                        (!busy).then_some(Message::FollowDecision(
                            group.ids[0].clone(),
                            actor.id.clone(),
                            false
                        ))
                    ),
                ]
                .spacing(8),
            );
        }
    } else if group.post.is_none() {
        if let Some(actor) = group.actors.first() {
            body = body.push(
                widget::button::standard(fl!("activity-view-profile"))
                    .on_press(Message::Status(status::Message::OpenAccount(actor.clone()))),
            );
        }
    }
    if busy {
        body = body.push(
            widget::row![
                widget::indeterminate_circular().size(18.),
                widget::text::caption(fl!("activity-updating"))
            ]
            .spacing(8),
        );
    }
    if let Some(error) = group.ids.iter().find_map(|id| inbox.action_errors.get(id)) {
        body = body.push(widget::text(error.clone()));
    }
    widget::container(body)
        .padding(16)
        .width(Length::Fill)
        .class(cosmic::style::Container::Card)
        .into()
}
pub fn event_label(kind: &Kind) -> String {
    match kind {
        Kind::Mention => fl!("activity-mentioned"),
        Kind::Favourite => fl!("activity-favorited"),
        Kind::Reblog => fl!("activity-boosted"),
        Kind::Follow => fl!("activity-followed"),
        Kind::FollowRequest => fl!("activity-requested"),
        Kind::PollVote => fl!("activity-poll-vote"),
        Kind::PollExpired => fl!("activity-poll-ended"),
        Kind::Status => fl!("activity-posted"),
        Kind::Update | Kind::QuotedUpdate => fl!("activity-edited"),
        Kind::Quote => fl!("activity-quoted"),
        Kind::AdminSignup => fl!("activity-signup"),
        Kind::AdminReport => fl!("activity-report"),
        Kind::Move => fl!("activity-moved"),
        Kind::Reaction => fl!("activity-reacted"),
        Kind::GroupInvited => fl!("activity-invited"),
        Kind::App => fl!("activity-app"),
        Kind::Unknown => fl!("activity-generic"),
    }
}
fn event_icon(kind: &Kind) -> &'static str {
    match kind {
        Kind::Mention | Kind::Quote => "mail-message-new-symbolic",
        Kind::Favourite | Kind::Reaction => "starred-symbolic",
        Kind::Reblog => "emblem-shared-symbolic",
        Kind::Follow | Kind::FollowRequest => "system-users-symbolic",
        Kind::PollVote | Kind::PollExpired => "view-list-symbolic",
        Kind::AdminReport => "dialog-warning-symbolic",
        _ => "dialog-information-symbolic",
    }
}
