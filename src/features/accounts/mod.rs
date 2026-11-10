//! Account profile view: bio/stats plus follow/mute/block relationship actions.

use cosmic::{
    app::Task,
    iced::{self, alignment::Horizontal, ContentFit, Length},
    widget, Element,
};
use megalodon::entities::{Account, Relationship};

use crate::app;
use crate::cache::Cache;
use crate::features::{reading::html, status};
use crate::fl;

#[derive(Debug, Clone)]
pub enum Message {
    Open(String),
    People(String, bool),
    EditProfile,
    Private(Account),
    Report(Account),
    Follow(String, bool),
    Mute(String, bool),
    Block(String, bool),
}

pub fn account<'a>(account: &'a Account, cache: &'a Cache) -> Element<'a, Message> {
    let spacing = cosmic::theme::active().cosmic().spacing;
    let handles = &cache.handles;
    let relationship = cache.relationships.get(&account.id);
    let is_me = cache.is_me(&account.id);

    let header = handles.get(&account.header).map(|handle| {
        widget::image(handle)
            .content_fit(ContentFit::Cover)
            .height(120.0)
    });
    let avatar = handles.get(&account.avatar).map(|handle| {
        widget::container(
            widget::button::image(handle)
                .on_press(Message::Open(account.avatar.clone()))
                .width(100),
        )
        .center(Length::Fill)
    });
    let stack = iced::widget::stack!(header, avatar);
    let display_name = widget::text(&account.display_name).size(18);
    let username =
        widget::button::link(html::full_acct(account)).on_press(Message::Open(account.url.clone()));
    let bio = (!account.note.is_empty()).then(|| html::view(&account.note, Message::Open));
    let joined = widget::text::caption(format!(
        "Joined on {}",
        account.created_at.format("%d %b %Y")
    ));
    let fields: Vec<Element<_>> = account
        .fields
        .iter()
        .map(|field| {
            widget::column![
                widget::text(field.name.clone()),
                html::view(&field.value, Message::Open)
            ]
            .width(Length::Fill)
            .into()
        })
        .collect();
    let followers = widget::column![
        widget::button::link(crate::fl!("profile-followers"))
            .on_press(Message::People(account.id.clone(), false)),
        widget::text::title3(account.followers_count.to_string()),
    ];
    let following = widget::column![
        widget::button::link(crate::fl!("profile-following"))
            .on_press(Message::People(account.id.clone(), true)),
        widget::text::title3(account.following_count.to_string()),
    ]
    .width(Length::FillPortion(1))
    .align_x(Horizontal::Center);
    let statuses = widget::column![
        widget::text::text(crate::fl!("profile-posts")),
        widget::text::title3(account.statuses_count.to_string()),
    ]
    .width(Length::FillPortion(1))
    .align_x(Horizontal::Center);

    let info = widget::container(
        widget::row![
            followers,
            widget::divider::vertical::light().height(Length::Fixed(50.)),
            following,
            widget::divider::vertical::light().height(Length::Fixed(50.)),
            statuses,
        ]
        .padding(spacing.space_xs)
        .spacing(spacing.space_xs),
    )
    .class(cosmic::style::Container::Card);

    let can_act = cache.me.is_some() && !is_me;
    let relationship_actions = can_act.then(|| relationship_actions(account, relationship));

    let settings = (!fields.is_empty()).then_some(widget::settings::section().extend(fields));
    let content = widget::column![
        stack,
        display_name,
        username,
        relationship_actions,
        is_me.then(|| widget::button::standard(fl!("profile-edit")).on_press(Message::EditProfile)),
        can_act.then(|| widget::button::standard(fl!("private-write"))
            .on_press(Message::Private(account.clone()))),
        bio,
        joined,
        info,
        settings,
        can_act.then(|| widget::button::standard(fl!("reading-report"))
            .on_press(Message::Report(account.clone())))
    ]
    .align_x(Horizontal::Center)
    .width(Length::Fill)
    .spacing(spacing.space_xs);

    widget::container(
        widget::settings::flex_item_row(vec![content.into()]).padding(spacing.space_xs),
    )
    .width(Length::Fill)
    .padding(spacing.space_s)
    .class(cosmic::style::Container::Card)
    .into()
}

fn relationship_actions<'a>(
    account: &'a Account,
    relationship: Option<&'a Relationship>,
) -> Element<'a, Message> {
    let spacing = cosmic::theme::active().cosmic().spacing;

    let following = relationship.is_some_and(|r| r.following);
    let muting = relationship.is_some_and(|r| r.muting);
    let blocking = relationship.is_some_and(|r| r.blocking);

    widget::row![
        widget::button::text(if following {
            crate::fl!("action-unfollow")
        } else {
            crate::fl!("action-follow")
        })
        .class(if following {
            cosmic::theme::Button::Standard
        } else {
            cosmic::theme::Button::Suggested
        })
        .on_press(Message::Follow(account.id.clone(), following)),
        widget::button::text(if muting {
            crate::fl!("action-unmute")
        } else {
            crate::fl!("action-mute")
        })
        .on_press(Message::Mute(account.id.clone(), muting)),
        widget::button::text(if blocking {
            crate::fl!("action-unblock")
        } else {
            crate::fl!("action-block")
        })
        .class(cosmic::theme::Button::Destructive)
        .on_press(Message::Block(account.id.clone(), blocking)),
    ]
    .spacing(spacing.space_xs)
    .into()
}

/// Handles [`Message::Open`] directly; [`Message::Follow`]/[`Message::Mute`]/
/// [`Message::Block`] carry no view-only behavior of their own — they're
/// intercepted in [`app::AppModel::update`] to perform the actual API call.
pub fn update(message: Message) -> Task<app::Message> {
    match message {
        Message::Open(url) => cosmic::task::message(app::Message::Open(url)),
        Message::Report(account) => {
            cosmic::task::message(app::Message::Status(status::Message::Report(account, None)))
        }
        _ => Task::none(),
    }
}
