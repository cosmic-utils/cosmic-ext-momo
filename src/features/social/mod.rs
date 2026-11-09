//! Profile editing, follower browsers, private conversations, and scheduled posts.
use crate::{
    app,
    cache::Cache,
    client::{within_timeout, Client},
    fl,
};
use cosmic::{app::Task, widget, Element};
use megalodon::{
    entities::{Account, Conversation, ScheduledStatus},
    megalodon::*,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    People(String, bool),
    EditProfile,
    Conversations,
    Scheduled,
}
impl Page {
    pub fn title(&self) -> String {
        match self {
            Self::People(_, true) => fl!("profile-following"),
            Self::People(_, false) => fl!("profile-followers"),
            Self::EditProfile => fl!("profile-edit"),
            Self::Conversations => fl!("private-conversations"),
            Self::Scheduled => fl!("scheduled-posts"),
        }
    }
}
#[derive(Debug, Clone)]
pub enum Data {
    People(Vec<Account>, Option<String>),
    Conversations(Vec<Conversation>, Option<String>),
    Scheduled(Vec<ScheduledStatus>, Option<String>),
    Profile(Account),
}
#[derive(Debug, Clone)]
pub enum Message {
    Refresh,
    More,
    Loaded(u64, Result<Data, String>),
    Name(String),
    BioAction(widget::text_editor::Action),
    Locked(bool),
    ReadConversation(u64, Result<Conversation, String>),
    Save,
    Saved(u64, Result<Account, String>),
    OpenConversation(String, String),
    ConfirmCancel(String),
    Cancel,
    CancelScheduled(String),
    Cancelled(u64, String, Result<(), String>),
    RescheduleText(String),
    Reschedule(String),
    Rescheduled(u64, Result<ScheduledStatus, String>),
    ConversationUpdated,
}
pub struct Social {
    page: Page,
    serial: u64,
    pub busy: bool,
    error: Option<String>,
    cursor: Option<String>,
    more: bool,
    people: Vec<Account>,
    conversations: Vec<Conversation>,
    scheduled: Vec<ScheduledStatus>,
    name: String,
    bio: String,
    bio_editor: widget::text_editor::Content,
    bio_dirty: bool,
    saved: bool,
    locked: bool,
    profile_ready: bool,
    older: bool,
    confirm: Option<String>,
    schedule: String,
}
impl Default for Social {
    fn default() -> Self {
        Self {
            page: Page::Conversations,
            serial: 0,
            busy: false,
            error: None,
            cursor: None,
            more: false,
            people: vec![],
            conversations: vec![],
            scheduled: vec![],
            name: String::new(),
            bio: String::new(),
            bio_editor: widget::text_editor::Content::default(),
            bio_dirty: false,
            saved: false,
            locked: false,
            profile_ready: false,
            older: false,
            confirm: None,
            schedule: String::new(),
        }
    }
}
impl Social {
    pub fn open(&mut self, page: Page, client: Client) -> Task<app::Message> {
        let serial = self.serial + 1;
        *self = Self {
            page,
            serial,
            ..Default::default()
        };
        self.load(client, false)
    }
    fn load(&mut self, client: Client, older: bool) -> Task<app::Message> {
        self.busy = true;
        self.error = None;
        self.serial += 1;
        self.older = older;
        let cursor = self.cursor.clone().filter(|_| older);
        let page = self.page.clone();
        let serial = self.serial;
        cosmic::task::future(async move {
            app::Message::Social(Message::Loaded(serial, fetch(client, page, cursor).await))
        })
    }
    pub fn view<'a>(&'a self, cache: &'a Cache) -> Element<'a, app::Message> {
        let mut body = widget::column![]
            .spacing(12)
            .width(cosmic::iced::Length::Fill);
        if let Some(error) = &self.error {
            body = body.push(widget::text(error)).push(
                widget::button::standard(fl!("reading-retry"))
                    .on_press_maybe((!self.busy).then_some(app::Message::Social(Message::Refresh))),
            );
        }
        if self.saved {
            body = body.push(widget::text::caption(fl!("profile-saved")));
        }
        if self.page == Page::EditProfile {
            let name = widget::text_input(fl!("profile-display-name"), &self.name);
            let bio = widget::text_editor::text_editor(&self.bio_editor)
                .placeholder(fl!("profile-bio"))
                .height(160);
            let locked = widget::toggler(self.locked);
            body = body
                .push(widget::text(fl!("profile-display-name")))
                .push(if self.busy {
                    name
                } else {
                    name.on_input(|value| app::Message::Social(Message::Name(value)))
                })
                .push(widget::text(fl!("profile-bio")))
                .push(if self.busy {
                    bio
                } else {
                    bio.on_action(|action| app::Message::Social(Message::BioAction(action)))
                })
                .push(widget::settings::item(
                    fl!("profile-locked"),
                    if self.busy {
                        locked
                    } else {
                        locked.on_toggle(|value| app::Message::Social(Message::Locked(value)))
                    },
                ))
                .push(
                    widget::button::suggested(fl!("profile-save")).on_press_maybe(
                        (!self.busy && self.profile_ready)
                            .then_some(app::Message::Social(Message::Save)),
                    ),
                );
        } else {
            body = body.push(
                widget::button::standard(fl!("feed-refresh"))
                    .on_press_maybe((!self.busy).then_some(app::Message::Social(Message::Refresh))),
            );
        }
        for person in &self.people {
            body = body.push(
                widget::button::link(crate::features::reading::html::full_acct(person)).on_press(
                    app::Message::ToggleContextPage(app::ContextPage::Account(person.clone())),
                ),
            );
        }
        if self.page == Page::Conversations {
            body = body.push(widget::text(fl!("private-help")));
        }
        for conversation in &self.conversations {
            let names = conversation
                .accounts
                .iter()
                .map(crate::features::reading::html::full_acct)
                .collect::<Vec<_>>()
                .join(", ");
            body = body.push(widget::text::heading(names));
            if conversation.unread {
                body = body.push(widget::text::caption(fl!("activity-unread")));
            }
            if let Some(post) = &conversation.last_status {
                body = body
                    .push(
                        crate::features::status::status(
                            post,
                            crate::features::status::StatusOptions::all()
                                .context(crate::features::reading::filters::Context::Thread),
                            cache,
                        )
                        .map(app::Message::Status),
                    )
                    .push(widget::button::standard(fl!("conversation-open")).on_press(
                        app::Message::Social(Message::OpenConversation(
                            conversation.id.clone(),
                            post.id.clone(),
                        )),
                    ));
            }
        }
        for scheduled in &self.scheduled {
            body = body
                .push(widget::text(
                    scheduled
                        .scheduled_at
                        .with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M %Z")
                        .to_string(),
                ))
                .push(widget::selectable_text(scheduled.params.text.clone()));
            if self.confirm.as_ref() == Some(&scheduled.id) {
                body = body
                    .push(widget::text(fl!("schedule-cancel-confirm")))
                    .push(
                        widget::button::destructive(fl!("schedule-cancel")).on_press_maybe(
                            (!self.busy).then_some(app::Message::Social(Message::CancelScheduled(
                                scheduled.id.clone(),
                            ))),
                        ),
                    )
                    .push(
                        widget::text_input(fl!("schedule-placeholder"), &self.schedule)
                            .on_input(|value| app::Message::Social(Message::RescheduleText(value))),
                    )
                    .push(
                        widget::button::standard(fl!("schedule-change")).on_press_maybe(
                            (!self.busy).then_some(app::Message::Social(Message::Reschedule(
                                scheduled.id.clone(),
                            ))),
                        ),
                    )
                    .push(widget::button::standard(fl!("cancel")).on_press_maybe(
                        (!self.busy).then_some(app::Message::Social(Message::Cancel)),
                    ));
            } else {
                body = body.push(
                    widget::button::standard(fl!("schedule-manage")).on_press_maybe(
                        (!self.busy).then_some(app::Message::Social(Message::ConfirmCancel(
                            scheduled.id.clone(),
                        ))),
                    ),
                );
            }
        }
        if self.busy {
            body = body.push(widget::indeterminate_circular().size(24.));
        }
        if self.more {
            body = body.push(
                widget::button::standard(fl!("reading-load-more"))
                    .on_press_maybe((!self.busy).then_some(app::Message::Social(Message::More))),
            );
        }
        widget::scrollable(body).into()
    }
    pub fn update(&mut self, message: Message, client: Client) -> Task<app::Message> {
        match message {
            Message::Refresh if !self.busy => return self.load(client, false),
            Message::More if !self.busy && self.more => return self.load(client, true),
            Message::ConversationUpdated if self.page == Page::Conversations && !self.busy => {
                return self.load(client, false)
            }
            Message::Loaded(serial, result) if serial == self.serial => {
                self.busy = false;
                match result {
                    Err(error) => self.error = Some(error),
                    Ok(data) => {
                        if !self.older {
                            self.people.clear();
                            self.conversations.clear();
                            self.scheduled.clear();
                        }
                        let mut posts = vec![];
                        let next = match data {
                            Data::People(accounts, next) => {
                                for account in accounts {
                                    if !self.people.iter().any(|old| old.id == account.id) {
                                        self.people.push(account);
                                    }
                                }
                                next
                            }
                            Data::Conversations(items, next) => {
                                for item in items {
                                    if let Some(post) = &item.last_status {
                                        posts.push(post.clone());
                                    }
                                    if !self.conversations.iter().any(|old| old.id == item.id) {
                                        self.conversations.push(item);
                                    }
                                }
                                next
                            }
                            Data::Scheduled(items, next) => {
                                for item in items {
                                    if !self.scheduled.iter().any(|old| old.id == item.id) {
                                        self.scheduled.push(item);
                                    }
                                }
                                next
                            }
                            Data::Profile(account) => {
                                self.profile_ready = true;
                                self.bio_dirty = false;
                                self.name = account.display_name;
                                self.bio = account
                                    .source
                                    .as_ref()
                                    .map(|source| source.note.clone())
                                    .unwrap_or_default();
                                self.bio_editor =
                                    widget::text_editor::Content::with_text(&self.bio);
                                self.locked = account.locked;
                                None
                            }
                        };
                        self.more = next.is_some() && (!self.older || next != self.cursor);
                        self.cursor = next;
                        return Task::batch(
                            posts
                                .into_iter()
                                .map(|post| cosmic::task::message(app::Message::CacheStatus(post))),
                        );
                    }
                }
            }
            Message::Name(value) if !self.busy => {
                self.name = value;
                self.saved = false;
            }
            Message::BioAction(action) if !self.busy && self.profile_ready => {
                let editing = action.is_edit();
                self.bio_editor.perform(action);
                if editing {
                    self.bio = self.bio_editor.text();
                    self.bio_dirty = true;
                    self.saved = false;
                }
            }
            Message::Locked(value) if !self.busy => {
                self.locked = value;
                self.saved = false;
            }
            Message::Save if !self.busy && self.profile_ready && self.page == Page::EditProfile => {
                self.busy = true;
                self.error = None;
                let serial = self.serial;
                let options = UpdateCredentialsInputOptions {
                    display_name: Some(self.name.clone()),
                    note: self.bio_dirty.then(|| self.bio.clone()),
                    locked: Some(self.locked),
                    ..Default::default()
                };
                return cosmic::task::future(async move {
                    app::Message::Social(Message::Saved(
                        serial,
                        within_timeout(
                            client.update_credentials(Some(&options)),
                            fl!("action-error"),
                        )
                        .await
                        .map(|response| response.json),
                    ))
                });
            }
            Message::Saved(serial, result) if serial == self.serial => {
                self.busy = false;
                match result {
                    Ok(account) => {
                        self.saved = true;
                        self.error = None;
                        return cosmic::task::message(app::Message::SetAccount(account));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::OpenConversation(id, post) => {
                let serial = self.serial;
                return Task::batch([
                    cosmic::task::message(app::Message::ToggleContextPage(
                        app::ContextPage::Status(post),
                    )),
                    cosmic::task::future(async move {
                        app::Message::Social(Message::ReadConversation(
                            serial,
                            within_timeout(client.read_conversation(id), fl!("action-error"))
                                .await
                                .map(|response| response.json),
                        ))
                    }),
                ]);
            }
            Message::ReadConversation(serial, result) if serial == self.serial => match result {
                Ok(conversation) => {
                    if let Some(old) = self
                        .conversations
                        .iter_mut()
                        .find(|old| old.id == conversation.id)
                    {
                        *old = conversation;
                    }
                }
                Err(error) => self.error = Some(error),
            },
            Message::ConfirmCancel(id) if !self.busy => {
                self.confirm = Some(id);
                self.schedule.clear();
            }
            Message::Cancel if !self.busy => self.confirm = None,
            Message::RescheduleText(value) if !self.busy => self.schedule = value,
            Message::CancelScheduled(id) if !self.busy && self.confirm.as_ref() == Some(&id) => {
                self.busy = true;
                let serial = self.serial;
                return cosmic::task::future(async move {
                    let result = within_timeout(
                        client.cancel_scheduled_status(id.clone()),
                        fl!("action-error"),
                    )
                    .await
                    .map(|_| ());
                    app::Message::Social(Message::Cancelled(serial, id, result))
                });
            }
            Message::Cancelled(serial, id, result) if serial == self.serial => {
                self.busy = false;
                match result {
                    Ok(()) => {
                        self.scheduled.retain(|post| post.id != id);
                        self.confirm = None;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::Reschedule(id) if !self.busy && self.confirm.as_ref() == Some(&id) => {
                let Some(time) = crate::features::compose::schedule_time(&self.schedule) else {
                    self.error = Some(fl!("schedule-invalid"));
                    return Task::none();
                };
                self.busy = true;
                let serial = self.serial;
                return cosmic::task::future(async move {
                    app::Message::Social(Message::Rescheduled(
                        serial,
                        within_timeout(client.schedule_status(id, Some(time)), fl!("action-error"))
                            .await
                            .map(|response| response.json),
                    ))
                });
            }
            Message::Rescheduled(serial, result) if serial == self.serial => {
                self.busy = false;
                match result {
                    Ok(item) => {
                        self.scheduled.retain(|old| old.id != item.id);
                        self.scheduled.push(item);
                        self.confirm = None;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            _ => (),
        }
        Task::none()
    }
}
fn next<T>(response: &megalodon::response::Response<Vec<T>>) -> Option<String> {
    response
        .header
        .get("link")
        .and_then(|value| value.to_str().ok())
        .and_then(crate::client::next_cursor)
}
async fn fetch(client: Client, page: Page, cursor: Option<String>) -> Result<Data, String> {
    match page {
        Page::People(id, following) => {
            let options = AccountFollowersInputOptions {
                limit: Some(20),
                max_id: cursor,
                ..Default::default()
            };
            let response = if following {
                within_timeout(
                    client.get_account_following(id, Some(&options)),
                    fl!("feed-load-error"),
                )
                .await?
            } else {
                within_timeout(
                    client.get_account_followers(id, Some(&options)),
                    fl!("feed-load-error"),
                )
                .await?
            };
            Ok(Data::People(response.json.clone(), next(&response)))
        }
        Page::EditProfile => Ok(Data::Profile(
            within_timeout(
                client.verify_account_credentials(),
                fl!("session-load-error"),
            )
            .await?
            .json,
        )),
        Page::Conversations => {
            let options = GetConversationTimelineInputOptions {
                limit: Some(20),
                max_id: cursor,
                ..Default::default()
            };
            let response = within_timeout(
                client.get_conversation_timeline(Some(&options)),
                fl!("feed-load-error"),
            )
            .await?;
            Ok(Data::Conversations(response.json.clone(), next(&response)))
        }
        Page::Scheduled => {
            let options = GetScheduledStatusesInputOptions {
                limit: Some(20),
                max_id: cursor,
                ..Default::default()
            };
            let response = within_timeout(
                client.get_scheduled_statuses(Some(&options)),
                fl!("feed-load-error"),
            )
            .await?;
            Ok(Data::Scheduled(response.json.clone(), next(&response)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn client() -> Client {
        Client::new("https://example.org".into(), None)
    }
    fn account(id: &str) -> Account {
        serde_json::from_value(serde_json::json!({"id":id,"username":"alice","acct":"alice","display_name":"Alice","locked":false,"created_at":"2020-01-01T00:00:00Z","followers_count":0,"following_count":0,"statuses_count":0,"note":"","url":"https://example.org/@alice","avatar":"","avatar_static":"","header":"","header_static":"","emojis":[],"fields":[],"bot":false})).unwrap()
    }
    #[test]
    fn stale_people_pages_cannot_replace_a_new_browser_and_refresh_failures_keep_rows() {
        let mut social = Social::default();
        let _ = social.open(Page::People("one".into(), false), client());
        let old = social.serial;
        let _ = social.open(Page::People("two".into(), true), client());
        let _ = social.update(
            Message::Loaded(
                old,
                Ok(Data::People(vec![account("stale")], Some("old".into()))),
            ),
            client(),
        );
        assert!(social.people.is_empty() && social.busy);
        let _ = social.update(
            Message::Loaded(
                social.serial,
                Ok(Data::People(vec![account("1")], Some("cursor".into()))),
            ),
            client(),
        );
        assert!(social.more);
        let _ = social.update(Message::More, client());
        let _ = social.update(
            Message::Loaded(
                social.serial,
                Ok(Data::People(
                    vec![account("1"), account("2")],
                    Some("cursor".into()),
                )),
            ),
            client(),
        );
        assert_eq!(social.people.len(), 2);
        assert!(!social.more);
        let _ = social.update(Message::Refresh, client());
        let _ = social.update(
            Message::Loaded(social.serial, Err("offline".into())),
            client(),
        );
        assert_eq!(social.people.len(), 2);
        assert!(!social.busy);
    }
    #[test]
    fn profile_save_requires_loaded_credentials_and_scheduled_cancellation_needs_confirmation() {
        let mut social = Social {
            page: Page::EditProfile,
            ..Default::default()
        };
        let _ = social.update(Message::Save, client());
        assert!(!social.busy);
        let _ = social.open(Page::Scheduled, client());
        social.busy = false;
        let _ = social.update(Message::CancelScheduled("1".into()), client());
        assert!(!social.busy);
        let _ = social.update(Message::ConfirmCancel("1".into()), client());
        let _ = social.update(Message::CancelScheduled("2".into()), client());
        assert!(!social.busy);
        let _ = social.update(Message::CancelScheduled("1".into()), client());
        assert!(social.busy);
        let _ = social.update(
            Message::Cancelled(social.serial.wrapping_sub(1), "1".into(), Ok(())),
            client(),
        );
        assert!(social.busy);
        let _ = social.update(
            Message::Cancelled(social.serial, "1".into(), Err("offline".into())),
            client(),
        );
        assert!(!social.busy);
        assert_eq!(social.confirm.as_deref(), Some("1"));
    }
}
