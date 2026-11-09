//! Account-scoped, recoverable posting and editing.
pub mod count;
pub mod network;
pub mod store;
#[cfg(test)]
mod tests;

use crate::{app, features::reading, fl};
use cosmic::{iced::Length, widget};
use megalodon::entities::{status::StatusVisibility, Account, Attachment, Status};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicU64, Arc},
};

#[derive(Debug, Clone)]
pub struct Limits {
    pub characters: usize,
    pub reserved_url: usize,
    pub attachments: usize,
    pub image_bytes: u64,
    pub description: usize,
    pub image_mimes: Vec<String>,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            characters: 500,
            reserved_url: 23,
            attachments: 4,
            image_bytes: 16 * 1024 * 1024,
            description: 1500,
            image_mimes: vec![
                "image/jpeg".into(),
                "image/png".into(),
                "image/gif".into(),
                "image/webp".into(),
            ],
        }
    }
}
impl Limits {
    pub fn from_json(value: &serde_json::Value) -> Self {
        let mut limits = Self::default();
        let config = &value["configuration"];
        if let Some(v) = config["statuses"]["max_characters"].as_u64() {
            limits.characters = v as usize;
        }
        if let Some(v) = config["statuses"]["characters_reserved_per_url"].as_u64() {
            limits.reserved_url = v as usize;
        }
        if let Some(v) = config["statuses"]["max_media_attachments"].as_u64() {
            limits.attachments = v as usize;
        }
        if let Some(v) = config["media_attachments"]["image_size_limit"].as_u64() {
            limits.image_bytes = v;
        }
        if let Some(v) = config["media_attachments"]["description_limit"].as_u64() {
            limits.description = v as usize;
        }
        if let Some(values) = config["media_attachments"]["supported_mime_types"].as_array() {
            limits.image_mimes = values
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
        }
        limits
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Media {
    pub id: String,
    pub name: String,
    pub stored_name: Option<String>,
    pub bytes: u64,
    pub description: String,
    pub attachment: Option<Attachment>,
    #[serde(skip)]
    pub uploading: bool,
    #[serde(skip)]
    pub progress: Arc<AtomicU64>,
    #[serde(skip)]
    pub error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct State {
    pub id: String,
    pub text: String,
    pub in_reply_to_id: Option<String>,
    pub edit_id: Option<String>,
    pub content_warning: bool,
    pub spoiler_text: String,
    pub visibility: StatusVisibility,
    pub reply_ceiling: Option<StatusVisibility>,
    pub sensitive: bool,
    pub language: Option<String>,
    pub media: Vec<Media>,
    pub had_media: bool,
    pub has_poll: bool,
    pub dirty: bool,
    pub submission_key: String,
    pub attempted_at: Option<i64>,
    pub published_id: Option<String>,
    #[serde(skip)]
    pub sending: bool,
    #[serde(skip)]
    pub importing: bool,
    #[serde(skip)]
    pub error: Option<String>,
    #[serde(skip)]
    pub storage_error: Option<String>,
}
impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compose")
            .field("id", &self.id)
            .field("edit_id", &self.edit_id)
            .finish_non_exhaustive()
    }
}
impl Default for State {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            text: String::new(),
            in_reply_to_id: None,
            edit_id: None,
            content_warning: false,
            spoiler_text: String::new(),
            visibility: StatusVisibility::Public,
            reply_ceiling: None,
            sensitive: false,
            language: None,
            media: vec![],
            had_media: false,
            has_poll: false,
            dirty: false,
            submission_key: uuid::Uuid::new_v4().to_string(),
            attempted_at: None,
            published_id: None,
            sending: false,
            importing: false,
            error: None,
            storage_error: None,
        }
    }
}
impl State {
    pub fn new(account: Option<&Account>) -> Self {
        let mut state = Self::default();
        if account.is_some() {
            state.visibility = StatusVisibility::Private;
        }
        if let Some(source) = account.and_then(|account| account.source.as_ref()) {
            state.visibility = source
                .privacy
                .as_deref()
                .and_then(|value| value.parse().ok())
                .unwrap_or(StatusVisibility::Private);
            state.sensitive = source.sensitive.unwrap_or(false);
            state.language = source.language.clone();
        }
        state
    }
    pub fn reply_to(post: &Status, me: Option<&Account>) -> Self {
        let mut state = Self::new(me);
        state.in_reply_to_id = Some(post.id.clone());
        state.reply_ceiling = Some(post.visibility.clone());
        if visibility_rank(&state.visibility) < visibility_rank(&post.visibility) {
            state.visibility = post.visibility.clone();
        }
        state.spoiler_text = post.spoiler_text.clone();
        state.content_warning = !state.spoiler_text.is_empty();
        state.language = post.language.clone().or(state.language);
        let mut recipients = vec![];
        if !me.is_some_and(|me| me.id == post.account.id) {
            recipients.push(reading::html::full_acct(&post.account));
        }
        let mut recipient_ids = std::collections::HashSet::from([post.account.id.clone()]);
        for mention in &post.mentions {
            if !recipient_ids.insert(mention.id.clone()) || me.is_some_and(|me| me.id == mention.id)
            {
                continue;
            }
            let host = reqwest::Url::parse(&mention.url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_string));
            let acct = if mention.acct.contains('@') {
                format!("@{}", mention.acct)
            } else if let Some(host) = host {
                format!("@{}@{}", mention.acct, host)
            } else {
                format!("@{}", mention.acct)
            };
            if !recipients
                .iter()
                .any(|value| value.eq_ignore_ascii_case(&acct))
            {
                recipients.push(acct);
            }
        }
        if !recipients.is_empty() {
            state.text = format!("{} ", recipients.join(" "));
        }
        state
    }
    pub fn edit(post: &Status, source: megalodon::entities::StatusSource) -> Self {
        let mut state = Self::default();
        state.edit_id = Some(post.id.clone());
        state.in_reply_to_id = post.in_reply_to_id.clone();
        state.text = source.text;
        state.spoiler_text = source.spoiler_text;
        state.content_warning = !state.spoiler_text.is_empty();
        state.visibility = post.visibility.clone();
        state.sensitive = post.sensitive;
        state.language = post.language.clone();
        state.had_media = !post.media_attachments.is_empty();
        state.has_poll = post.poll.is_some();
        state.media = post
            .media_attachments
            .iter()
            .map(|attachment| Media {
                id: uuid::Uuid::new_v4().to_string(),
                name: attachment.r#type.to_string(),
                stored_name: None,
                bytes: 0,
                description: attachment.description.clone().unwrap_or_default(),
                attachment: Some(attachment.clone()),
                uploading: false,
                progress: Arc::default(),
                error: None,
            })
            .collect();
        state
    }
    pub fn busy(&self) -> bool {
        self.sending || self.importing || self.media.iter().any(|media| media.uploading)
    }
    /// Whether metadata (alt text, uploads) may change; unlike `editable`, ignores running uploads.
    pub fn accepts_edits(&self) -> bool {
        !self.sending
            && !self.importing
            && self.attempted_at.is_none()
            && self.published_id.is_none()
    }
    pub fn editable(&self) -> bool {
        self.accepts_edits() && !self.media.iter().any(|media| media.uploading)
    }
    pub fn retry_safe(&self) -> bool {
        self.edit_id.is_some()
            || self
                .attempted_at
                .is_none_or(|time| chrono::Utc::now().timestamp() - time < 55 * 60)
    }
    pub fn changed(&mut self) {
        self.dirty = true;
        self.error = None;
        self.submission_key = uuid::Uuid::new_v4().to_string();
    }
    pub fn failed(&mut self, error: network::SubmitError) {
        self.sending = false;
        match error {
            network::SubmitError::Rejected(error) => {
                self.attempted_at = None;
                self.error = Some(error);
            }
            network::SubmitError::Uncertain => self.error = Some(fl!("compose-uncertain")),
        }
    }
    pub fn mixed_media(&self) -> bool {
        self.media.len() > 1
            && self.media.iter().any(|media| {
                media.attachment.as_ref().is_some_and(|attachment| {
                    attachment.r#type != megalodon::entities::attachment::AttachmentType::Image
                })
            })
    }
    pub fn characters(&self, limits: &Limits) -> usize {
        let warning = if self.content_warning {
            &self.spoiler_text
        } else {
            ""
        };
        count::count(&self.text, warning, limits.reserved_url)
    }
    pub fn valid(&self, limits: &Limits) -> bool {
        let text = &self.text;
        self.published_id.is_none()
            && (!text.trim().is_empty() || !self.media.is_empty() || self.has_poll)
            && !self.mixed_media()
            && !(self.has_poll && !self.media.is_empty())
            && self.media.len() <= limits.attachments
            && self.characters(limits) <= limits.characters
            && self.media.iter().all(|media| {
                media.attachment.is_some()
                    && !media.uploading
                    && media.error.is_none()
                    && media.description.chars().count() <= limits.description
            })
            && self
                .reply_ceiling
                .as_ref()
                .is_none_or(|ceiling| visibility_rank(&self.visibility) >= visibility_rank(ceiling))
    }
    pub fn payload(&self) -> serde_json::Value {
        let mut payload = serde_json::json!({"status":self.text, "spoiler_text":if self.content_warning { self.spoiler_text.as_str() } else { "" }, "sensitive":self.sensitive || self.content_warning});
        if self.edit_id.is_none() {
            payload["visibility"] = serde_json::json!(self.visibility);
            if let Some(id) = &self.in_reply_to_id {
                payload["in_reply_to_id"] = id.clone().into();
            }
        }
        if let Some(language) = &self.language {
            payload["language"] = language.clone().into();
        }
        // Existing polls are deliberately left unchanged by text edits.
        if !self.media.is_empty() || self.had_media {
            payload["media_ids"] = serde_json::json!(self
                .media
                .iter()
                .filter_map(|media| media
                    .attachment
                    .as_ref()
                    .map(|attachment| attachment.id.clone()))
                .collect::<Vec<_>>());
            if self.edit_id.is_some() {
                payload["media_attributes"] = serde_json::json!(self.media.iter().filter_map(|media| media.attachment.as_ref().map(|attachment| serde_json::json!({"id":attachment.id,"description":media.description}))).collect::<Vec<_>>());
            }
        }
        payload
    }
}
pub fn visibility_rank(value: &StatusVisibility) -> u8 {
    match value {
        StatusVisibility::Public => 0,
        StatusVisibility::Unlisted | StatusVisibility::Local => 1,
        StatusVisibility::Private => 2,
        StatusVisibility::Direct => 3,
    }
}

#[derive(Clone)]
pub enum Message {
    New,
    Pane(widget::segmented_button::Entity),
    EditAlt(String),
    Reply(String),
    Edit(String),
    Resume(String),
    DeleteDraft(String),
    Drafts,
    Warning(bool),
    Spoiler(String),
    Visibility(StatusVisibility),
    Sensitive(bool),
    Language(String),
    PickImages,
    Picked(u64, Result<Vec<PathBuf>, String>),
    Imported(u64, Result<Vec<Media>, String>),
    Alt(String, String),
    Remove(String),
    Upload(String),
    Uploaded(u64, String, Result<Attachment, String>),
    Submit,
    Prepared(u64, Result<Vec<Attachment>, String>),
    Submitted(u64, Result<network::Published, network::SubmitError>),
    EditLoaded(u64, Result<State, String>),
    Close,
    Keep,
    Discard,
    ContinueEditing,
    Tick,
    AllowEditing,
}
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ComposeMessage")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Images,
    Warning,
    Settings,
}

pub fn panes() -> widget::segmented_button::SingleSelectModel {
    let mut model = widget::segmented_button::SingleSelectModel::default();
    model
        .insert()
        .text(fl!("compose-images"))
        .icon(widget::icon::from_name("insert-image-symbolic"))
        .data(Pane::Images)
        .activate();
    model
        .insert()
        .text(fl!("content-warning"))
        .icon(widget::icon::from_name("dialog-warning-symbolic"))
        .data(Pane::Warning);
    model
        .insert()
        .text(fl!("compose-language-tab"))
        .icon(widget::icon::from_name("preferences-system-symbolic"))
        .data(Pane::Settings);
    model
}

#[allow(clippy::too_many_arguments)]
pub fn view<'a>(
    state: &'a State,
    editor: &'a widget::text_editor::Content,
    tabs: &'a widget::segmented_button::SingleSelectModel,
    reply_preview: Option<cosmic::Element<'a, app::Message>>,
    limits: &Limits,
    account_label: String,
    client: &crate::client::Client,
    store: &store::Store,
) -> widget::Dialog<'a, app::Message> {
    let editable = state.editable();
    let mut top = widget::column![].spacing(16).width(Length::Fill);
    if let Some(preview) = reply_preview {
        top = top.push(
            widget::container(
                widget::column![
                    widget::text::caption(fl!("compose-replying")),
                    widget::container(widget::scrollable(preview)).max_height(110),
                ]
                .spacing(8),
            )
            .padding(12)
            .width(Length::Fill)
            .class(cosmic::style::Container::Card),
        );
    } else if state.in_reply_to_id.is_some() {
        top = top.push(widget::text::caption(fl!("compose-replying")));
    }
    let variants: Vec<_> = [
        StatusVisibility::Public,
        StatusVisibility::Unlisted,
        StatusVisibility::Private,
        StatusVisibility::Direct,
    ]
    .into_iter()
    .filter(|value| {
        state
            .reply_ceiling
            .as_ref()
            .is_none_or(|ceiling| visibility_rank(value) >= visibility_rank(ceiling))
    })
    .collect();
    let audience: cosmic::Element<'a, app::Message> = if state.edit_id.is_none() && editable {
        let labels = variants.iter().map(visibility_label).collect::<Vec<_>>();
        let selected = variants.iter().position(|value| value == &state.visibility);
        widget::dropdown(labels, selected, move |index| {
            app::Message::Compose(Message::Visibility(variants[index].clone()))
        })
        .into()
    } else {
        widget::text(visibility_label(&state.visibility)).into()
    };
    let header = widget::row![
        widget::text::caption(format!("{} {account_label}", fl!("compose-posting-as")))
            .width(Length::Fill),
        widget::text::caption(fl!("compose-audience")),
        audience,
    ]
    .spacing(8)
    .align_y(cosmic::iced::Alignment::Center);
    if state.content_warning {
        let input = widget::text_input(fl!("content-warning-placeholder"), &state.spoiler_text);
        top = top.push(if editable {
            input.on_input(|value| app::Message::Compose(Message::Spoiler(value)))
        } else {
            input
        });
    }
    let text_editor = widget::text_editor::text_editor(editor)
        .placeholder(fl!("whats-happening"))
        .size(16)
        .font(cosmic::iced::Font::DEFAULT)
        .style(|theme, status| {
            let mut style = <cosmic::Theme as widget::text_editor::Catalog>::style(
                theme,
                &<cosmic::Theme as widget::text_editor::Catalog>::default(),
                status,
            );
            // Use the theme's foreground: the upstream editor uses a fixed palette swatch.
            style.value = theme.cosmic().on_bg_color().into();
            style.placeholder = cosmic::iced::Color {
                a: 0.6,
                ..style.value
            };
            style.border.width = 1.0;
            style.border.radius = 8.0.into();
            style
        })
        .height(Length::Fill)
        .padding(12);
    top = top.push(if editable {
        text_editor.on_action(app::Message::EditorAction)
    } else {
        text_editor
    });
    let remaining = limits.characters as i64 - state.characters(limits) as i64;

    let pane: cosmic::Element<'a, app::Message> = match tabs.active_data::<Pane>() {
        Some(Pane::Warning) => {
            let toggle = widget::toggler(state.content_warning);
            let toggle = if editable {
                toggle.on_toggle(|value| app::Message::Compose(Message::Warning(value)))
            } else {
                toggle
            };
            let sensitive = widget::toggler(state.sensitive);
            let sensitive = if editable {
                sensitive.on_toggle(|value| app::Message::Compose(Message::Sensitive(value)))
            } else {
                sensitive
            };
            widget::settings::section()
                .add(widget::settings::item(fl!("compose-sensitive"), sensitive))
                .add(widget::settings::item(fl!("content-warning"), toggle))
                .into()
        }
        Some(Pane::Settings) => {
            let language = widget::text_input(
                fl!("compose-language-placeholder"),
                state.language.as_deref().unwrap_or(""),
            );
            let language = if editable {
                language.on_input(|value| app::Message::Compose(Message::Language(value)))
            } else {
                language
            };
            widget::column![
                widget::text::caption(fl!("compose-language")),
                language,
                widget::text::caption(fl!("compose-language-help")),
            ]
            .spacing(8)
            .into()
        }
        _ => {
            let add = widget::button::icon(widget::icon::from_name("list-add-symbolic"))
                .icon_size(32)
                .padding(20)
                .class(cosmic::style::Button::Standard)
                .on_press_maybe(
                    (editable && !state.has_poll && state.media.len() < limits.attachments)
                        .then_some(app::Message::Compose(Message::PickImages)),
                );
            #[cfg(target_os = "linux")]
            let add = add.name(fl!("compose-add-images"));
            let mut images = widget::row![add].spacing(12);
            for media in &state.media {
                images = images.push(media_card(state, media, editable, client, store));
            }
            widget::scrollable(images)
                .direction(cosmic::iced::widget::scrollable::Direction::Horizontal(
                    cosmic::iced::widget::scrollable::Scrollbar::default(),
                ))
                .into()
        }
    };
    let pane = widget::container(pane)
        .padding(12)
        .width(Length::Fill)
        .class(cosmic::style::Container::Card);
    let tabs = widget::segmented_control::horizontal(tabs)
        .on_activate(|entity| app::Message::Compose(Message::Pane(entity)));
    top = top.push(tabs).push(pane);

    let mut controls = widget::column![].spacing(16).width(Length::Fill);
    if state.mixed_media() {
        controls = controls.push(widget::text(fl!("compose-mixed-media")));
    }
    if let Some(error) = &state.error {
        controls = controls.push(widget::text(error));
    }
    if let Some(error) = &state.storage_error {
        controls = controls.push(widget::text(error));
    }
    if state.published_id.is_some() {
        controls = controls.push(widget::text(fl!("compose-posted")));
    }
    if state.importing {
        controls = controls
            .push(widget::indeterminate_circular().size(24.))
            .push(widget::text(fl!("compose-preparing-images")));
    }
    if state.sending {
        controls = controls
            .push(widget::indeterminate_circular().size(24.))
            .push(widget::text(fl!("compose-sending")));
    }
    if state.attempted_at.is_some() && !state.sending {
        controls = controls.push(widget::text(fl!("compose-uncertain")));
        if !state.retry_safe() {
            controls = controls
                .push(widget::text(fl!("compose-retry-expired")))
                .push(
                    widget::button::standard(fl!("compose-checked-profile"))
                        .on_press(app::Message::Compose(Message::AllowEditing)),
                );
        }
    }
    let has_extras = state.mixed_media()
        || state.error.is_some()
        || state.storage_error.is_some()
        || state.published_id.is_some()
        || state.importing
        || state.sending
        || state.attempted_at.is_some();
    let extras: cosmic::Element<'a, app::Message> = if has_extras {
        widget::container(widget::scrollable(controls).height(Length::Shrink))
            .max_height(120)
            .into()
    } else {
        widget::space::vertical().height(0).into()
    };

    let count = if remaining < 0 {
        format!("{} {}", -remaining, fl!("compose-over-limit"))
    } else {
        format!("{} {}", remaining, fl!("compose-remaining"))
    };
    let summary = [
        state
            .language
            .as_ref()
            .map(|language| format!("{}: {language}", fl!("compose-language"))),
        state.sensitive.then(|| fl!("compose-sensitive")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let footer = widget::row![
        widget::text::caption(summary).width(Length::Fill),
        widget::text::caption(count)
    ]
    .spacing(12);
    widget::dialog()
        .width(Length::Fill)
        .max_width(680)
        .height(Length::Fill)
        .max_height(680)
        .title(if state.edit_id.is_some() {
            fl!("compose-edit")
        } else if state.in_reply_to_id.is_some() {
            fl!("reply")
        } else {
            fl!("new-post")
        })
        .control(
            widget::column![header, top.height(Length::Fill), extras, footer]
                .spacing(12)
                .height(Length::Fill)
                .width(Length::Fill),
        )
        .primary_action(
            widget::button::suggested(if state.error.is_some() {
                fl!("compose-retry")
            } else if state.edit_id.is_some() {
                fl!("compose-save-edit")
            } else {
                fl!("post")
            })
            .on_press_maybe(
                (!state.busy() && state.retry_safe() && state.valid(limits))
                    .then_some(app::Message::Compose(Message::Submit)),
            ),
        )
        .secondary_action(
            widget::button::standard(fl!("compose-close"))
                .on_press_maybe((!state.busy()).then_some(app::Message::Compose(Message::Close))),
        )
}

fn media_card<'a>(
    state: &State,
    media: &'a Media,
    editable: bool,
    client: &crate::client::Client,
    store: &store::Store,
) -> cosmic::Element<'a, app::Message> {
    let preview: cosmic::Element<'a, app::Message> = preview_path(state, media, client, store)
        .map(|path| {
            widget::image(path)
                .width(72)
                .height(72)
                .content_fit(cosmic::iced::ContentFit::Cover)
                .into()
        })
        .unwrap_or_else(|| {
            widget::container(widget::icon::from_name("insert-image-symbolic").size(32))
                .center(72)
                .into()
        });
    let thumbnail = widget::button::custom(preview)
        .padding(0)
        .class(cosmic::style::Button::Image)
        .on_press_maybe(
            editable.then(|| app::Message::Compose(Message::EditAlt(media.id.clone()))),
        );
    let overlay: Option<cosmic::Element<'a, app::Message>> = if media.uploading {
        Some(widget::indeterminate_circular().size(32.).into())
    } else if media.error.is_some() {
        Some(
            widget::icon::from_name("dialog-error-symbolic")
                .size(32)
                .into(),
        )
    } else {
        None
    };
    let mut layers = vec![thumbnail.into()];
    if let Some(overlay) = overlay {
        layers.push(widget::container(overlay).center(72).into());
    }
    widget::column![
        cosmic::iced::widget::Stack::with_children(layers)
            .width(72)
            .height(72),
        crate::ui::icon_action(
            "window-close-symbolic",
            fl!("compose-remove"),
            editable.then(|| app::Message::Compose(Message::Remove(media.id.clone()))),
        )
    ]
    .spacing(4)
    .align_x(cosmic::iced::Alignment::Center)
    .into()
}

pub fn draft_card<'a>(draft: &'a State) -> cosmic::Element<'a, app::Message> {
    let text = draft.text.trim();
    let title = if !draft.spoiler_text.trim().is_empty() && draft.content_warning {
        draft.spoiler_text.trim()
    } else {
        text
    };
    let title = if title.is_empty() {
        fl!("compose-untitled")
    } else {
        title.replace('\n', " ").chars().take(140).collect()
    };
    let mut details = vec![if draft.edit_id.is_some() {
        fl!("compose-edit")
    } else if draft.in_reply_to_id.is_some() {
        fl!("reply")
    } else {
        fl!("new-post")
    }];
    details.push(visibility_label(&draft.visibility));
    if !draft.media.is_empty() {
        details.push(format!("{} {}", draft.media.len(), fl!("compose-images")));
    }
    if draft.content_warning {
        details.push(fl!("content-warning"));
    }
    let id = draft.id.clone();
    widget::container(
        widget::column![
            widget::text::body(title).width(Length::Fill),
            widget::text::caption(details.join(" · ")),
            widget::row![
                widget::space::horizontal(),
                widget::button::destructive(fl!("compose-discard-short"))
                    .on_press(app::Message::Compose(Message::DeleteDraft(id.clone()))),
                widget::button::suggested(fl!("compose-continue-draft"))
                    .on_press(app::Message::Compose(Message::Resume(id))),
            ]
            .spacing(8),
        ]
        .spacing(8),
    )
    .padding(12)
    .width(Length::Fill)
    .class(cosmic::style::Container::Card)
    .into()
}

pub fn alt_input_id() -> widget::Id {
    widget::Id::new("compose-alt-input")
}

pub fn alt_view<'a>(
    state: &'a State,
    media_id: &str,
    limits: &Limits,
    client: &crate::client::Client,
    store: &store::Store,
) -> widget::Dialog<'a, app::Message> {
    let id = media_id.to_string();
    let mut content = widget::column![].spacing(12);
    if let Some(media) = state.media.iter().find(|media| media.id == id) {
        if let Some(path) = preview_path(state, media, client, store) {
            content = content.push(widget::image(path).height(200));
        }
        let input = widget::text_input(fl!("compose-alt"), &media.description)
            .id(alt_input_id())
            .on_input({
                let id = id.clone();
                move |value| app::Message::Compose(Message::Alt(id.clone(), value))
            });
        content = content.push(input).push(widget::text::caption(format!(
            "{}/{}",
            media.description.chars().count(),
            limits.description
        )));
    }
    widget::dialog()
        .title(fl!("compose-alt"))
        .control(content)
        .primary_action(
            widget::button::suggested(fl!("compose-done"))
                .on_press(app::Message::Dialog(app::DialogAction::Close)),
        )
}
fn visibility_label(value: &StatusVisibility) -> String {
    match value {
        StatusVisibility::Public => fl!("compose-public"),
        StatusVisibility::Unlisted => fl!("compose-unlisted"),
        StatusVisibility::Private => fl!("compose-followers"),
        StatusVisibility::Direct => fl!("compose-direct"),
        StatusVisibility::Local => fl!("compose-local"),
    }
}
fn preview_path(
    state: &State,
    media: &Media,
    client: &crate::client::Client,
    store: &store::Store,
) -> Option<PathBuf> {
    let name = media.stored_name.as_ref()?;
    let lower = media.name.to_lowercase();
    if ![".png", ".jpg", ".jpeg"]
        .iter()
        .any(|ext| lower.ends_with(ext))
    {
        return None;
    }
    store.asset_path(&client.cache_key(), &state.id, name).ok()
}
