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
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
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
    pub text: Option<String>,
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
            text: Some(String::new()),
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
        let mut recipient_ids = std::collections::HashSet::new();
        recipient_ids.insert(post.account.id.clone());
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
        state.text = Some(if recipients.is_empty() {
            String::new()
        } else {
            format!("{} ", recipients.join(" "))
        });
        state
    }
    pub fn edit(post: &Status, source: megalodon::entities::StatusSource) -> Self {
        let mut state = Self::default();
        state.edit_id = Some(post.id.clone());
        state.in_reply_to_id = post.in_reply_to_id.clone();
        state.text = Some(source.text);
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
    pub fn editable(&self) -> bool {
        !self.busy() && self.attempted_at.is_none() && self.published_id.is_none()
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
    pub fn valid(&self, limits: &Limits) -> bool {
        let text = self.text.as_deref().unwrap_or("");
        self.published_id.is_none()
            && (!text.trim().is_empty() || !self.media.is_empty() || self.has_poll)
            && !self.mixed_media()
            && !(self.has_poll && !self.media.is_empty())
            && self.media.len() <= limits.attachments
            && count::count(
                text,
                if self.content_warning {
                    &self.spoiler_text
                } else {
                    ""
                },
                limits.reserved_url,
            ) <= limits.characters
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
        let mut payload = serde_json::json!({"status":self.text.as_deref().unwrap_or(""), "spoiler_text":if self.content_warning { self.spoiler_text.as_str() } else { "" }, "sensitive":self.sensitive || self.content_warning});
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
    Reply(String),
    Edit(String),
    Resume(String),
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

pub fn view<'a>(
    state: &'a State,
    editor: &'a widget::text_editor::Content,
    reply_preview: Option<cosmic::Element<'a, app::Message>>,
    limits: &Limits,
    account_label: String,
    client: &crate::client::Client,
    store: &store::Store,
) -> widget::Dialog<'a, app::Message> {
    let editable = state.editable();
    let mut controls = widget::column![widget::text::heading(account_label)].spacing(12);
    if let Some(preview) = reply_preview {
        controls = controls.push(preview);
    }
    if let Some(id) = &state.in_reply_to_id {
        controls = controls.push(widget::text::caption(format!(
            "{} {}",
            fl!("compose-in-reply-to"),
            id
        )));
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
    if state.edit_id.is_some() {
        controls = controls.push(widget::text(format!(
            "{}: {}",
            fl!("compose-visibility"),
            state.visibility
        )));
    } else if editable {
        let labels = variants.iter().map(visibility_label).collect::<Vec<_>>();
        let selected = variants.iter().position(|value| value == &state.visibility);
        controls = controls.push(widget::dropdown(labels, selected, move |index| {
            app::Message::Compose(Message::Visibility(variants[index].clone()))
        }));
    } else {
        controls = controls.push(widget::text(state.visibility.to_string()));
    }
    let warning = widget::toggler(state.content_warning).label(fl!("content-warning"));
    controls = controls.push(if editable {
        warning.on_toggle(|value| app::Message::Compose(Message::Warning(value)))
    } else {
        warning
    });
    if state.content_warning {
        let input = widget::text_input(fl!("content-warning-placeholder"), &state.spoiler_text);
        controls = controls.push(if editable {
            input.on_input(|value| app::Message::Compose(Message::Spoiler(value)))
        } else {
            input
        });
    }
    let text_editor = widget::text_editor::text_editor(editor)
        .placeholder(fl!("whats-happening"))
        .height(160.)
        .padding(8);
    controls = controls.push(if editable {
        text_editor.on_action(app::Message::EditorAction)
    } else {
        text_editor
    });
    let remaining = limits.characters as i64
        - count::count(
            state.text.as_deref().unwrap_or(""),
            if state.content_warning {
                &state.spoiler_text
            } else {
                ""
            },
            limits.reserved_url,
        ) as i64;
    controls = controls.push(widget::text::caption(remaining.to_string()));
    let language = widget::text_input(
        fl!("compose-language"),
        state.language.as_deref().unwrap_or(""),
    );
    controls = controls.push(if editable {
        language.on_input(|value| app::Message::Compose(Message::Language(value)))
    } else {
        language
    });
    let sensitive = widget::toggler(state.sensitive).label(fl!("compose-sensitive"));
    controls = controls.push(if editable {
        sensitive.on_toggle(|value| app::Message::Compose(Message::Sensitive(value)))
    } else {
        sensitive
    });
    for media in &state.media {
        let mut item = widget::column![widget::text(media.name.clone())].spacing(8);
        if let Some(path) = media
            .stored_name
            .as_ref()
            .and_then(|name| store.asset_path(&client.cache_key(), &state.id, name).ok())
        {
            if name_is_static_image(&media.name) {
                item = item.push(widget::image(path).width(96).height(96));
            }
        }
        let alt = widget::text_input(fl!("compose-alt"), &media.description);
        let id = media.id.clone();
        item = item.push(if editable {
            alt.on_input(move |value| app::Message::Compose(Message::Alt(id.clone(), value)))
        } else {
            alt
        });
        item = item.push(widget::text::caption(format!(
            "{}/{}",
            media.description.chars().count(),
            limits.description
        )));
        if media.uploading {
            let percent = if media.bytes == 0 {
                0.
            } else {
                (media.progress.load(Ordering::Relaxed) as f32 / media.bytes as f32 * 100.)
                    .min(100.)
            };
            item = item
                .push(widget::determinate_linear(percent / 100.0))
                .push(widget::text(if percent >= 100. {
                    fl!("compose-processing")
                } else {
                    format!("{} {:.0}%", fl!("compose-uploading"), percent)
                }));
        } else if media.attachment.is_some() {
            item = item.push(widget::text::caption(fl!("compose-uploaded")));
        }
        if let Some(error) = &media.error {
            item = item.push(widget::text(error));
        }
        if editable {
            let mut actions = widget::row![widget::button::standard(fl!("compose-remove"))
                .on_press(app::Message::Compose(Message::Remove(media.id.clone())))]
            .spacing(8);
            if media.stored_name.is_some() {
                actions = actions.push(
                    widget::button::standard(if media.attachment.is_some() {
                        fl!("compose-reupload")
                    } else {
                        fl!("compose-upload")
                    })
                    .on_press(app::Message::Compose(Message::Upload(media.id.clone()))),
                );
            }
            item = item.push(actions);
        }
        controls = controls.push(item);
    }
    if state.mixed_media() {
        controls = controls.push(widget::text(fl!("compose-mixed-media")));
    }
    if editable && !state.has_poll && state.media.len() < limits.attachments {
        controls = controls.push(
            widget::button::standard(fl!("compose-add-images"))
                .on_press(app::Message::Compose(Message::PickImages)),
        );
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
    widget::dialog()
        .title(if state.edit_id.is_some() {
            fl!("compose-edit")
        } else if state.in_reply_to_id.is_some() {
            fl!("reply")
        } else {
            fl!("new-post")
        })
        .control(
            widget::container(widget::scrollable(controls))
                .height(420.)
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
fn visibility_label(value: &StatusVisibility) -> String {
    match value {
        StatusVisibility::Public => fl!("compose-public"),
        StatusVisibility::Unlisted => fl!("compose-unlisted"),
        StatusVisibility::Private => fl!("compose-followers"),
        StatusVisibility::Direct => fl!("compose-direct"),
        StatusVisibility::Local => fl!("compose-local"),
    }
}
fn name_is_static_image(name: &str) -> bool {
    let name = name.to_lowercase();
    name.ends_with(".png") || name.ends_with(".jpg") || name.ends_with(".jpeg")
}
