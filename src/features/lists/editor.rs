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
    Found(u64, Result<Vec<Account>, String>),
    Members(u64, Result<(Vec<Account>, Option<String>), String>),
    More,
    RetryMembers,
    Save,
    Saved(u64, Result<Option<List>, String>),
    ConfirmDelete,
    CancelDelete,
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
    query: String,
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
            title,
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
    pub fn view(&self) -> Element<'_, app::Message> {
        let mut body = widget::column![widget::button::standard(fl!("reading-back"))
            .on_press_maybe((!self.busy).then_some(message(Message::Close)))]
        .spacing(12);
        let title = widget::text_input(fl!("list-title"), &self.title);
        body = body
            .push(if self.busy {
                title
            } else {
                title.on_input(|value| message(Message::Title(value)))
            })
            .push(widget::button::suggested(fl!("list-save")).on_press_maybe(
                (!self.busy && !self.title.trim().is_empty()).then_some(message(Message::Save)),
            ));
        if self.id.is_some() {
            body = body.push(widget::text::heading(fl!("list-members")));
            for account in &self.members {
                body = body.push(
                    widget::row![
                        widget::text(crate::features::reading::html::full_acct(account))
                            .width(cosmic::iced::Length::Fill),
                        widget::button::standard(fl!("settings-remove")).on_press_maybe(
                            (!self.busy).then_some(message(Message::Remove(account.id.clone())))
                        )
                    ]
                    .spacing(8),
                );
            }
            if self.more {
                body = body.push(
                    widget::button::standard(fl!("reading-load-more"))
                        .on_press_maybe((!self.busy).then_some(message(Message::More))),
                );
            }
            let input = widget::text_input(fl!("list-search-following"), &self.query);
            body = body
                .push(if self.busy {
                    input
                } else {
                    input
                        .on_input(|value| message(Message::Query(value)))
                        .on_submit(|_| message(Message::Search))
                })
                .push(
                    widget::button::standard(fl!("search")).on_press_maybe(
                        (!self.busy && !self.query.trim().is_empty())
                            .then_some(message(Message::Search)),
                    ),
                );
            for account in &self.found {
                let member = self.members.iter().any(|old| old.id == account.id);
                body = body.push(
                    widget::row![
                        widget::text(crate::features::reading::html::full_acct(account))
                            .width(cosmic::iced::Length::Fill),
                        widget::button::standard(fl!("list-add-member")).on_press_maybe(
                            (!self.busy && !member)
                                .then_some(message(Message::Add(account.id.clone())))
                        )
                    ]
                    .spacing(8),
                );
            }
            if self.confirm {
                body =
                    body.push(widget::text(fl!("list-delete-confirm")))
                        .push(
                            widget::button::destructive(fl!("list-delete"))
                                .on_press_maybe((!self.busy).then_some(message(Message::Delete))),
                        )
                        .push(widget::button::standard(fl!("cancel")).on_press_maybe(
                            (!self.busy).then_some(message(Message::CancelDelete)),
                        ));
            } else {
                body = body.push(
                    widget::button::destructive(fl!("list-delete"))
                        .on_press_maybe((!self.busy).then_some(message(Message::ConfirmDelete))),
                );
            }
        }
        if self.busy {
            body = body.push(widget::indeterminate_circular().size(24.));
        }
        if let Some(error) = &self.error {
            body = body.push(widget::text(error));
            if self.id.is_some() {
                body = body.push(
                    widget::button::standard(fl!("reading-retry"))
                        .on_press_maybe((!self.busy).then_some(message(Message::RetryMembers))),
                );
            }
        }
        widget::scrollable(body).into()
    }
    pub fn update(&mut self, event: Message, client: Client) -> Task<app::Message> {
        match event {
            Message::Title(value) if !self.busy => self.title = value,
            Message::Query(value) if !self.busy => {
                self.query = value;
                self.found.clear();
                self.serial += 1;
            }
            Message::RetryMembers if !self.busy => return self.load(client, false),
            Message::More if !self.busy && self.more => return self.load(client, true),
            Message::Search if !self.busy && !self.query.trim().is_empty() => {
                self.busy = true;
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
                match result {
                    Ok(accounts) => {
                        self.found = accounts;
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::Members(serial, result) if serial == self.serial => {
                self.busy = false;
                match result {
                    Ok((members, next)) => {
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
            Message::CancelDelete if !self.busy => self.confirm = false,
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
                    Ok(()) => return self.load(client, false),
                    Err(error) => self.error = Some(error),
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
