//! Confirmed list mutations and a following-only member chooser.
use crate::{
    app,
    client::{within_timeout, Client},
    fl,
};
use cosmic::{app::Task, widget, Element};
use megalodon::{
    entities::{Account, List},
    megalodon::{GetAccountsInListInputOptions, SearchAccountInputOptions},
};
#[derive(Debug, Clone)]
pub enum Message {
    Title(String),
    Query(String),
    Search,
    EnterSearch,
    ExitSearch,
    Found(u64, Result<Vec<Account>, String>),
    Members(u64, Result<(Vec<Account>, Option<String>), String>),
    More,
    RetryMembers,
    Save,
    Saved(u64, Result<Option<List>, String>),
    ConfirmDelete,
    Delete,
    Add(String),
    Remove(String),
    Changed(u64, Result<(), String>),
    Close,
}
pub struct Editor {
    pub id: Option<String>,
    pub serial: u64,
    title: String,
    original_title: String,
    query: String,
    search_mode: bool,
    search_loading: bool,
    members_loading: bool,
    searched: bool,
    pending_member: Option<(String, bool)>,
    membership: std::collections::HashMap<String, bool>,
    members: Vec<Account>,
    found: Vec<Account>,
    cursor: Option<String>,
    more: bool,
    busy: bool,
    error: Option<String>,
    confirm: bool,
    older: bool,
}
fn message(message: Message) -> app::Message {
    app::Message::Lists(super::Message::Editor(message))
}
impl Editor {
    pub fn new(id: Option<String>, title: String, serial: u64) -> Self {
        Self {
            id,
            serial,
            original_title: title.clone(),
            title,
            search_mode: false,
            search_loading: false,
            members_loading: false,
            searched: false,
            pending_member: None,
            membership: std::collections::HashMap::new(),
            query: String::new(),
            members: vec![],
            found: vec![],
            cursor: None,
            more: false,
            busy: false,
            error: None,
            confirm: false,
            older: false,
        }
    }
    pub fn load(&mut self, client: Client, older: bool) -> Task<app::Message> {
        let Some(id) = self.id.clone() else {
            return Task::none();
        };
        self.busy = true;
        self.members_loading = true;
        self.serial += 1;
        let serial = self.serial;
        self.older = older;
        let cursor = self.cursor.clone().filter(|_| older);
        cosmic::task::future(async move {
            let options = GetAccountsInListInputOptions {
                limit: Some(40),
                max_id: cursor,
                ..Default::default()
            };
            message(Message::Members(
                serial,
                within_timeout(
                    client.get_accounts_in_list(id, Some(&options)),
                    fl!("action-error"),
                )
                .await
                .map(|response| {
                    let next = response
                        .header
                        .get("link")
                        .and_then(|value| value.to_str().ok())
                        .and_then(crate::client::next_cursor);
                    (response.json, next)
                }),
            ))
        })
    }
    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn creation_dialog(&self) -> Element<'_, app::Message> {
        let input = widget::text_input(fl!("list-title"), &self.title).id(create_name_id());
        let mut content = widget::column![if self.busy {
            input
        } else {
            input
                .on_input(|value| message(Message::Title(value)))
                .on_submit(|_| message(Message::Save))
        }]
        .spacing(12);
        if self.busy {
            content = content.push(
                widget::row![
                    widget::indeterminate_circular().size(18.),
                    widget::text(fl!("list-creating"))
                ]
                .spacing(8),
            );
        }
        if let Some(error) = &self.error {
            content = content.push(widget::text(error));
        }
        widget::dialog()
            .title(fl!("list-create"))
            .body(fl!("list-create-description"))
            .control(content)
            .primary_action(
                widget::button::suggested(fl!("list-create")).on_press_maybe(
                    (!self.busy && !self.title.trim().is_empty()).then_some(message(Message::Save)),
                ),
            )
            .secondary_action(
                widget::button::standard(fl!("cancel"))
                    .on_press_maybe((!self.busy).then_some(message(Message::Close))),
            )
            .into()
    }

    pub fn view<'a>(&'a self, cache: &'a crate::cache::Cache) -> Element<'a, app::Message> {
        if self.confirm {
            return self.delete_dialog();
        }
        let title = widget::text_input(fl!("list-title"), &self.title);
        let mut body = widget::column![
            widget::text::caption(fl!("list-title")),
            if self.busy {
                title
            } else {
                title.on_input(|value| message(Message::Title(value)))
            }
        ]
        .spacing(12);
        if self.search_mode {
            body = body.push(
                widget::button::text(fl!("reading-back"))
                    .leading_icon(widget::icon::from_name("go-previous-symbolic"))
                    .on_press_maybe(
                        (!self.busy || self.search_loading || self.members_loading)
                            .then_some(message(Message::ExitSearch)),
                    ),
            );
            let input = widget::text_input(fl!("list-search-following"), &self.query);
            let input = if !self.busy || self.search_loading {
                input
                    .on_input(|value| message(Message::Query(value)))
                    .on_submit(|_| message(Message::Search))
            } else {
                input
            };
            body = body.push(
                widget::row![
                    input,
                    widget::button::standard(fl!("search")).on_press_maybe(
                        (!self.busy && !self.query.trim().is_empty())
                            .then_some(message(Message::Search))
                    )
                ]
                .spacing(8),
            );
            if self.found.is_empty() && !self.busy {
                body = body.push(widget::text::caption(if self.searched {
                    fl!("list-search-empty")
                } else {
                    fl!("list-search-help")
                }));
            }
            for account in &self.found {
                body = body.push(self.person(account, cache, true));
            }
        } else {
            body = body.push(
                widget::row![
                    widget::text::heading(fl!("list-members")),
                    widget::space::horizontal(),
                    widget::button::standard(fl!("list-add-people"))
                        .on_press_maybe((!self.busy).then_some(message(Message::EnterSearch)))
                ]
                .align_y(cosmic::iced::Alignment::Center)
                .spacing(8),
            );
            if self.members.is_empty() && !self.busy {
                body = body.push(widget::text::caption(fl!("list-members-empty")));
            }
            for account in &self.members {
                body = body.push(self.person(account, cache, false));
            }
            if self.more {
                body = body.push(
                    widget::button::standard(fl!("reading-load-more"))
                        .on_press_maybe((!self.busy).then_some(message(Message::More))),
                );
            }
        }
        if self.busy {
            body = body.push(widget::indeterminate_circular().size(24.));
        }
        if let Some(error) = &self.error {
            body = body.push(widget::text(error)).push(
                widget::button::standard(fl!("reading-retry")).on_press_maybe(
                    (!self.busy).then_some(message(if self.search_mode {
                        Message::Search
                    } else {
                        Message::RetryMembers
                    })),
                ),
            );
        }
        body = body.push(widget::text::caption(fl!("list-members-saved")));
        let renamed = self.title.trim() != self.original_title.trim();
        let mut dialog = widget::dialog()
            .title(fl!("list-edit-title"))
            .max_width(600)
            .control(widget::container(widget::scrollable(body)).max_height(420))
            .primary_action(
                widget::button::suggested(if renamed {
                    fl!("list-save-name")
                } else {
                    fl!("list-done")
                })
                .on_press_maybe(
                    (!self.busy && (!renamed || !self.title.trim().is_empty())).then_some(message(
                        if renamed {
                            Message::Save
                        } else {
                            Message::Close
                        },
                    )),
                ),
            );
        if renamed {
            dialog = dialog.secondary_action(
                widget::button::standard(fl!("cancel"))
                    .on_press_maybe((!self.busy).then_some(message(Message::Close))),
            );
        }
        dialog.into()
    }

    fn person<'a>(
        &'a self,
        account: &'a Account,
        cache: &'a crate::cache::Cache,
        search: bool,
    ) -> Element<'a, app::Message> {
        let member = self
            .membership
            .get(&account.id)
            .copied()
            .unwrap_or_else(|| self.members.iter().any(|member| member.id == account.id));
        let name = if account.display_name.trim().is_empty() {
            account.username.clone()
        } else {
            account.display_name.clone()
        };
        let avatar = widget::image(
            cache
                .handles
                .get(&account.avatar)
                .cloned()
                .unwrap_or_else(crate::cache::fallback_handle),
        )
        .width(40)
        .height(40)
        .content_fit(cosmic::iced::ContentFit::Cover)
        .border_radius([20.; 4]);
        widget::row![
            avatar,
            widget::column![
                widget::text(name),
                widget::text::caption(crate::features::reading::html::full_acct(account))
            ]
            .spacing(2)
            .width(cosmic::iced::Length::Fill),
            widget::button::standard(if search && !member {
                fl!("list-add-member")
            } else {
                fl!("settings-remove")
            })
            .on_press_maybe((!self.busy).then_some(message(if search && !member {
                Message::Add(account.id.clone())
            } else {
                Message::Remove(account.id.clone())
            })))
        ]
        .align_y(cosmic::iced::Alignment::Center)
        .spacing(12)
        .padding([8, 0])
        .into()
    }

    fn delete_dialog(&self) -> Element<'_, app::Message> {
        let mut content = widget::column![widget::text(&self.title)].spacing(12);
        if self.busy {
            content = content.push(widget::indeterminate_circular().size(24.));
        }
        if let Some(error) = &self.error {
            content = content.push(widget::text(error));
        }
        widget::dialog()
            .title(fl!("list-delete"))
            .body(fl!("list-delete-confirm"))
            .control(content)
            .primary_action(
                widget::button::destructive(fl!("list-delete"))
                    .on_press_maybe((!self.busy).then_some(message(Message::Delete))),
            )
            .secondary_action(
                widget::button::standard(fl!("cancel"))
                    .on_press_maybe((!self.busy).then_some(message(Message::Close))),
            )
            .into()
    }

    pub fn update(&mut self, event: Message, client: Client) -> Task<app::Message> {
        match event {
            Message::Title(value) if !self.busy => self.title = value,
            Message::EnterSearch if !self.busy => {
                self.search_mode = true;
                self.error = None;
            }
            Message::ExitSearch if !self.busy || self.search_loading || self.members_loading => {
                if self.search_loading {
                    self.serial += 1;
                    self.busy = false;
                    self.search_loading = false;
                }
                self.search_mode = false;
                self.query.clear();
                self.found.clear();
                self.error = None;
                self.searched = false;
            }
            Message::Query(value) if !self.busy || self.search_loading => {
                self.busy = false;
                self.search_loading = false;
                self.searched = false;
                self.error = None;
                self.query = value;
                self.found.clear();
                self.serial += 1;
            }
            Message::RetryMembers if !self.busy => return self.load(client, false),
            Message::More if !self.busy && self.more => return self.load(client, true),
            Message::Search if !self.busy && !self.query.trim().is_empty() => {
                self.busy = true;
                self.search_mode = true;
                self.search_loading = true;
                self.error = None;
                self.serial += 1;
                let serial = self.serial;
                let query = self.query.trim().to_owned();
                return cosmic::task::future(async move {
                    let options = SearchAccountInputOptions {
                        following: Some(true),
                        limit: Some(20),
                        ..Default::default()
                    };
                    message(Message::Found(
                        serial,
                        within_timeout(
                            client.search_account(query, Some(&options)),
                            fl!("search-error"),
                        )
                        .await
                        .map(|response| response.json),
                    ))
                });
            }
            Message::Found(serial, result) if serial == self.serial => {
                self.busy = false;
                self.search_loading = false;
                self.searched = true;
                match result {
                    Ok(accounts) => {
                        let avatars = avatars(&accounts);
                        self.found = accounts;
                        self.error = None;
                        return cosmic::task::message(app::Message::Fetch(avatars));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::Members(serial, result) if serial == self.serial => {
                self.busy = false;
                self.members_loading = false;
                match result {
                    Ok((members, next)) => {
                        let avatars = avatars(&members);
                        if !self.older {
                            self.members.clear();
                        }
                        for account in members {
                            if !self.members.iter().any(|old| old.id == account.id) {
                                self.members.push(account);
                            }
                        }
                        self.more = next.is_some() && (!self.older || next != self.cursor);
                        self.cursor = next;
                        self.error = None;
                        return cosmic::task::message(app::Message::Fetch(avatars));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::Save if !self.busy && !self.title.trim().is_empty() => {
                self.busy = true;
                self.serial += 1;
                let serial = self.serial;
                let title = self.title.trim().to_owned();
                let id = self.id.clone();
                return cosmic::task::future(async move {
                    let result = if let Some(id) = id {
                        within_timeout(client.update_list(id, title), fl!("action-error")).await
                    } else {
                        within_timeout(client.create_list(title), fl!("action-error")).await
                    };
                    message(Message::Saved(
                        serial,
                        result.map(|response| Some(response.json)),
                    ))
                });
            }
            Message::ConfirmDelete if !self.busy => self.confirm = true,
            Message::Delete if !self.busy && self.confirm && self.id.is_some() => {
                self.busy = true;
                self.serial += 1;
                let serial = self.serial;
                let id = self.id.clone().unwrap();
                return cosmic::task::future(async move {
                    message(Message::Saved(
                        serial,
                        within_timeout(client.delete_list(id), fl!("action-error"))
                            .await
                            .map(|_| None),
                    ))
                });
            }
            Message::Add(ref account) | Message::Remove(ref account)
                if !self.busy && self.id.is_some() =>
            {
                let adding = matches!(event, Message::Add(_));
                if adding && !self.found.iter().any(|found| &found.id == account) {
                    return Task::none();
                }
                self.busy = true;
                self.pending_member = Some((account.clone(), adding));
                self.error = None;
                self.serial += 1;
                let serial = self.serial;
                let id = self.id.clone().unwrap();
                let account = account.clone();
                return cosmic::task::future(async move {
                    let result = membership(client, id, account, adding).await;
                    message(Message::Changed(serial, result))
                });
            }
            Message::Changed(serial, result) if serial == self.serial => {
                self.busy = false;
                match result {
                    Ok(()) => {
                        if let Some((id, adding)) = self.pending_member.take() {
                            self.membership.insert(id.clone(), adding);
                            if !adding {
                                self.members.retain(|member| member.id != id);
                            }
                        }
                        return self.load(client, false);
                    }
                    Err(error) => {
                        self.pending_member = None;
                        self.error = Some(error);
                    }
                }
            }
            Message::Saved(serial, Err(error)) if serial == self.serial => {
                self.busy = false;
                self.error = Some(error);
            }
            _ => (),
        }
        Task::none()
    }
}
async fn membership(
    client: Client,
    id: String,
    account: String,
    adding: bool,
) -> Result<(), String> {
    let mut url = reqwest::Url::parse(&client.base_url).map_err(|_| fl!("action-error"))?;
    url.path_segments_mut()
        .map_err(|_| fl!("action-error"))?
        .clear()
        .extend(["api", "v1", "lists", &id, "accounts"]);
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| fl!("action-error"))?;
    let request = if adding {
        http.post(url)
    } else {
        http.delete(url)
    };
    let response = request
        .bearer_auth(client.token.unwrap_or_default())
        .header("Content-Type", "application/json")
        .body(serde_json::json!({"account_ids":[account]}).to_string())
        .send()
        .await
        .map_err(|_| fl!("action-error"))?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(fl!("session-expired"));
    }
    if !response.status().is_success() {
        return Err(fl!("action-error"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn person() -> Account {
        serde_json::from_value(serde_json::json!({"id":"alice","username":"alice","acct":"alice","display_name":"Alice","locked":false,"created_at":"2020-01-01T00:00:00Z","followers_count":0,"following_count":0,"statuses_count":0,"note":"","url":"https://example.org/@alice","avatar":"https://example.org/avatar.png","avatar_static":"","header":"","header_static":"","emojis":[],"fields":[],"bot":false})).unwrap()
    }

    #[test]
    fn leaving_search_keeps_members_and_rejects_late_results() {
        let client = Client::new("https://example.org".into(), None);
        let mut editor = Editor::new(Some("one".into()), "Friends".into(), 0);
        editor.members = vec![person()];
        editor.cursor = Some("cursor".into());
        editor.more = true;
        let _ = editor.update(Message::EnterSearch, client.clone());
        let _ = editor.update(Message::Query("alice".into()), client.clone());
        let _ = editor.update(Message::Search, client.clone());
        let old = editor.serial;
        assert!(editor.search_mode && editor.search_loading);
        let _ = editor.update(Message::ExitSearch, client.clone());
        let _ = editor.update(Message::Found(old, Ok(vec![person()])), client);
        assert!(!editor.search_mode && !editor.busy && editor.found.is_empty());
        assert_eq!(editor.members.len(), 1);
        assert_eq!(editor.cursor.as_deref(), Some("cursor"));
        assert!(editor.more);
    }

    #[test]
    fn adding_a_search_result_preserves_results_and_remembers_confirmed_membership() {
        let client = Client::new("https://example.org".into(), None);
        let mut editor = Editor::new(Some("one".into()), "Friends".into(), 0);
        editor.search_mode = true;
        editor.found = vec![person()];
        let _ = editor.update(Message::Add("alice".into()), client.clone());
        let _ = editor.update(Message::Changed(editor.serial, Ok(())), client.clone());
        assert!(editor.search_mode && editor.members_loading);
        assert_eq!(editor.found.len(), 1);
        assert_eq!(editor.membership.get("alice"), Some(&true));
        let serial = editor.serial;
        let _ = editor.update(Message::ExitSearch, client.clone());
        assert!(!editor.search_mode);
        assert_eq!(editor.serial, serial);
        let _ = editor.update(Message::Members(serial, Ok((vec![person()], None))), client);
        assert!(!editor.busy);
        assert_eq!(editor.members.len(), 1);
    }

    #[test]
    fn deleting_a_list_requires_confirmation_and_does_not_hide_failures() {
        let client = Client::new("https://example.org".into(), None);
        let mut editor = Editor::new(Some("one".into()), "Friends".into(), 10);
        let _ = editor.update(Message::Delete, client.clone());
        assert!(!editor.busy);
        let _ = editor.update(Message::ConfirmDelete, client.clone());
        let _ = editor.update(Message::Delete, client.clone());
        assert!(editor.busy);
        let _ = editor.update(Message::Saved(10, Err("stale".into())), client.clone());
        assert!(editor.busy);
        let _ = editor.update(Message::Saved(editor.serial, Err("offline".into())), client);
        assert!(!editor.busy);
        assert_eq!(editor.id.as_deref(), Some("one"));
        assert_eq!(editor.error.as_deref(), Some("offline"));
    }
}

pub fn create_name_id() -> cosmic::iced::core::widget::Id {
    cosmic::iced::core::widget::Id::new("list-create-name")
}

fn avatars(accounts: &[Account]) -> Vec<String> {
    accounts
        .iter()
        .filter(|account| !account.avatar.is_empty())
        .map(|account| account.avatar.clone())
        .collect()
}
