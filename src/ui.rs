//! Shared layout and interaction conventions for the native COSMIC UI.
use cosmic::{
    iced::{Alignment, Length},
    widget, Element,
};

pub const CONTENT_WIDTH: u32 = 800;
pub const GAP: u16 = 16;

pub fn empty<'a, M: Clone + 'static>(
    icon: &str,
    title: String,
    description: String,
) -> Element<'a, M> {
    widget::container(
        widget::column![
            widget::icon::from_name(icon.to_string()).size(40),
            widget::text::title3(title),
            widget::text(description),
        ]
        .spacing(12)
        .align_x(Alignment::Center),
    )
    .padding(32)
    .center_x(Length::Fill)
    .into()
}
pub fn icon_action<'a, M: Clone + 'static>(
    icon: &str,
    label: String,
    message: Option<M>,
) -> Element<'a, M> {
    let button = widget::button::icon(widget::icon::from_name(icon.to_string()))
        .icon_size(18)
        .padding(8)
        .tooltip(label.clone())
        .on_press_maybe(message);
    #[cfg(target_os = "linux")]
    let button = button.name(label);
    button.into()
}

/// Keep the refresh control's footprint while a refresh is in progress.
pub fn refresh_action<'a, M: Clone + 'static>(
    refreshing: bool,
    label: String,
    message: Option<M>,
) -> Element<'a, M> {
    if refreshing {
        widget::container(widget::indeterminate_circular().size(18.))
            .padding(8)
            .into()
    } else {
        icon_action("view-refresh-symbolic", label, message)
    }
}

/// Match post surfaces without inheriting the accent colors or mouse-hover
/// focus border used by image buttons. Keyboard focus remains visible.
pub fn surface_button_style(highlight: bool) -> cosmic::style::Button {
    cosmic::style::Button::Custom {
        active: Box::new(|focused, theme| surface_style(theme, focused, false)),
        hovered: Box::new(move |focused, theme| surface_style(theme, focused, highlight)),
        pressed: Box::new(move |focused, theme| surface_style(theme, focused, highlight)),
        disabled: Box::new(|theme| surface_style(theme, false, false)),
    }
}

fn surface_style(theme: &cosmic::Theme, focused: bool, highlight: bool) -> widget::button::Style {
    use widget::button::Catalog;
    let mut style = theme.active(focused, false, &cosmic::style::Button::Image);
    style.text_color = None;
    style.icon_color = None;
    if highlight {
        style.background = Some(theme.current_container().component.hover.into());
    }
    style
}
