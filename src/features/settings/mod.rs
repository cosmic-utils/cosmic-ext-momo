//! Settings context page: timeline display preferences and account switching.

use cosmic::{widget, Element};

use crate::client::Session;
use crate::config::{FeedDensity, ThemeMode, TootConfig};
use crate::fl;

#[derive(Debug, Clone)]
pub enum Message {
    ToggleHideBoosts(bool),
    ToggleHideReplies(bool),
    SetFeedDensity(FeedDensity),
    SetThemeMode(ThemeMode),
    SwitchAccount(usize),
    RemoveAccount(usize),
    AddAccount,
    ConfirmClearCache,
    CancelClearCache,
    ClearCache,
    VerifySession,
}

pub fn view<'a>(
    config: &'a TootConfig,
    sessions: &'a [Session],
    active: usize,
    confirm_clear: bool,
    clearing: bool,
) -> Element<'a, Message> {
    let spacing = cosmic::theme::active().cosmic().spacing;

    let density_labels: Vec<String> = FeedDensity::ALL
        .iter()
        .map(|density| density.label())
        .collect();
    let density_selected = FeedDensity::ALL
        .iter()
        .position(|density| *density == config.feed_density);

    let theme_labels: Vec<String> = ThemeMode::ALL.iter().map(|mode| mode.label()).collect();
    let theme_selected = ThemeMode::ALL
        .iter()
        .position(|mode| *mode == config.theme_mode);

    let appearance_settings = widget::settings::section()
        .title(fl!("settings-appearance"))
        .add(widget::settings::item(
            fl!("settings-theme"),
            widget::column![widget::dropdown(theme_labels, theme_selected, |index| {
                Message::SetThemeMode(ThemeMode::ALL[index])
            }),]
            .spacing(spacing.space_xxs)
            .align_x(cosmic::iced::Alignment::End),
        ));

    let timeline_settings = widget::settings::section()
        .title(fl!("settings-timeline"))
        .add(widget::settings::item(
            fl!("settings-post-display"),
            widget::dropdown(density_labels, density_selected, |index| {
                Message::SetFeedDensity(FeedDensity::ALL[index])
            }),
        ))
        .add(widget::settings::item(
            fl!("settings-hide-boosts"),
            widget::toggler(config.hide_boosts).on_toggle(Message::ToggleHideBoosts),
        ))
        .add(widget::settings::item(
            fl!("settings-hide-replies"),
            widget::toggler(config.hide_replies).on_toggle(Message::ToggleHideReplies),
        ));

    let accounts_section = sessions
        .iter()
        .enumerate()
        .fold(
            widget::settings::section().title(fl!("settings-accounts")),
            |section, (index, session)| {
                let mut row: Vec<Element<'a, Message>> = vec![
                    Element::from(widget::text(session.handle())),
                    Element::from(widget::space::horizontal()),
                ];
                if index != active {
                    row.push(Element::from(
                        widget::button::standard(fl!("settings-switch"))
                            .on_press(Message::SwitchAccount(index)),
                    ));
                }
                row.push(Element::from(
                    widget::button::destructive(fl!("settings-remove"))
                        .on_press(Message::RemoveAccount(index)),
                ));
                section.add(widget::settings::item_row(row))
            },
        )
        .add(widget::button::suggested(fl!("setup-add-account")).on_press(Message::AddAccount));

    let mut cache_section = widget::settings::section()
        .title(fl!("settings-storage"))
        .add(widget::text(fl!("settings-cache-description")));
    if confirm_clear {
        cache_section = cache_section
            .add(widget::text(fl!("settings-cache-confirm")))
            .add(
                widget::row![
                    widget::button::destructive(fl!("settings-clear-cache"))
                        .on_press_maybe((!clearing).then_some(Message::ClearCache)),
                    widget::button::standard(fl!("cancel"))
                        .on_press_maybe((!clearing).then_some(Message::CancelClearCache))
                ]
                .spacing(8),
            );
    } else {
        cache_section = cache_section.add(
            widget::button::standard(fl!("settings-clear-cache"))
                .on_press(Message::ConfirmClearCache),
        );
    }
    if clearing {
        cache_section = cache_section.add(widget::text(fl!("settings-clearing-cache")));
    }
    let accounts_section = accounts_section.add(
        widget::button::standard(fl!("session-check"))
            .on_press_maybe((!sessions.is_empty()).then_some(Message::VerifySession)),
    );

    widget::column![
        appearance_settings,
        timeline_settings,
        accounts_section,
        cache_section
    ]
    .spacing(spacing.space_m)
    .into()
}
