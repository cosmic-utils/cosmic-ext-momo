//! Activity inbox: grouped reactions, explicit loading states, and confirmed
//! server mutations. New stream events are buffered to avoid moving the list
//! while somebody is reading it.
pub mod fetch;
pub mod model;
#[cfg(test)]
mod tests;
pub mod view;

use crate::{
    app,
    cache::{self, Cache},
    client::Client,
    features::{reading, status},
    fl, ui,
};
use cosmic::{
    app::Task,
    iced::{Length, Subscription},
    widget::{self, segmented_button},
    Element,
};
use megalodon::entities::{Notification, Relationship};
use model::{Filter, Tab};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadKind {
    Refresh,
    Older,
}
pub struct Notifications {
    pub mastodon: Client,
    notifications: VecDeque<String>,
    new: VecDeque<String>,
    max_id: Option<String>,
    loading: bool,
    has_loaded: bool,
    more: bool,
    request: u64,
    load_kind: LoadKind,
    error: Option<String>,
    filter: Filter,
    tabs: segmented_button::SingleSelectModel,
    pub(super) pending: HashSet<String>,
    dismissed: HashSet<String>,
    pub(super) expanded: HashSet<String>,
    pub(super) action_errors: HashMap<String, String>,
    pub(super) clearing: bool,
    confirm_clear: bool,
}
#[derive(Debug, Clone)]
pub enum Message {
    SetClient(Client),
    PrependNotification(Notification),
    Status(status::Message),
    Refresh,
    Retry,
    LoadMore,
    PageLoaded(u64, Result<fetch::Page, String>),
    SelectTab(segmented_button::Entity),
    SetFilter(Filter),
    ShowAll,
    PublishNew,
    TogglePeople(String),
    ConfirmClear,
    CancelClear,
    ClearAll,
    Cleared(Vec<String>, Result<(), String>),
    Dismiss(Vec<String>),
    Dismissed(Vec<(String, Result<(), String>)>),
    FollowDecision(String, String, bool),
    FollowFinished(String, Result<Relationship, String>),
}
fn cache_notification(notification: Notification) -> Task<app::Message> {
    Task::batch([
        cosmic::task::message(app::Message::Fetch(cache::extract_notification_images(
            &notification,
        ))),
        cosmic::task::message(app::Message::CacheNotification(notification)),
    ])
}
impl Notifications {
    pub fn new(mastodon: Client) -> Self {
        let mut tabs = segmented_button::SingleSelectModel::default();
        tabs.insert()
            .text(fl!("activity-all"))
            .data(Tab::All)
            .activate();
        tabs.insert()
            .text(fl!("activity-mentions"))
            .data(Tab::Mentions);
        tabs.insert()
            .text(fl!("activity-requests"))
            .data(Tab::Requests);
        Self {
            mastodon,
            notifications: VecDeque::new(),
            new: VecDeque::new(),
            max_id: None,
            loading: false,
            has_loaded: false,
            more: true,
            request: 0,
            load_kind: LoadKind::Refresh,
            error: None,
            filter: Filter::All,
            tabs,
            pending: HashSet::new(),
            dismissed: HashSet::new(),
            expanded: HashSet::new(),
            action_errors: HashMap::new(),
            clearing: false,
            confirm_clear: false,
        }
    }
    pub fn is_authenticated(&self) -> bool {
        self.mastodon.is_authenticated()
    }
    pub fn reset(&mut self, client: Client) {
        let request = self.request + 1;
        *self = Self::new(client);
        self.request = request;
    }
    fn tab(&self) -> Tab {
        self.tabs.active_data::<Tab>().copied().unwrap_or_default()
    }
    pub fn load_cached(&mut self) -> Task<app::Message> {
        if self.is_authenticated() && self.mastodon.account_id.is_empty() {
            return Task::none();
        }
        let cached: Vec<Notification> =
            crate::persistence::load_snapshot(&self.mastodon.cache_key(), "notifications");
        let mut tasks = vec![];
        for notification in cached {
            if !self.notifications.contains(&notification.id) {
                self.max_id = Some(notification.id.clone());
                self.notifications.push_back(notification.id.clone());
            }
            tasks.push(cache_notification(notification));
        }
        Task::batch(tasks)
    }
    pub fn save_cached(&self, cache: &Cache) {
        if self.is_authenticated() && self.mastodon.account_id.is_empty() {
            return;
        }
        let mut items: Vec<_> = self
            .new
            .iter()
            .chain(&self.notifications)
            .filter_map(|id| cache.notifications.get(id).cloned())
            .collect();
        items.sort_by_key(|n| std::cmp::Reverse(n.created_at));
        items.dedup_by(|a, b| a.id == b.id);
        crate::persistence::save_notification_snapshot(&self.mastodon.cache_key(), &items);
    }
    fn remove(&mut self, ids: &[String]) {
        self.dismissed.extend(ids.iter().cloned());
        self.notifications.retain(|id| !ids.contains(id));
        self.new.retain(|id| !ids.contains(id));
        for id in ids {
            self.pending.remove(id);
            self.action_errors.remove(id);
        }
    }
    fn publish_new(&mut self) {
        while let Some(id) = self.new.pop_back() {
            if !self.notifications.contains(&id) {
                self.notifications.push_front(id);
            }
        }
    }
    fn start_load(&mut self, kind: LoadKind) {
        self.request += 1;
        self.loading = true;
        self.load_kind = kind;
        self.error = None;
    }
    pub fn view<'a>(&'a self, cache: &'a Cache) -> Element<'a, Message> {
        let tab = self.tab();
        let groups = model::groups(
            self.notifications
                .iter()
                .filter_map(|id| cache.notifications.get(id)),
            cache,
            tab,
            self.filter,
        );
        let any = !self.notifications.is_empty() || !self.new.is_empty();
        let refreshing = self.loading && self.load_kind == LoadKind::Refresh;
        let mut content = widget::column![
            widget::row![
                widget::text::title2(fl!("notifications")),
                widget::space::horizontal(),
                ui::icon_action(
                    "view-refresh-symbolic",
                    fl!("activity-refresh"),
                    (!self.loading && !self.clearing).then_some(Message::Refresh)
                ),
                ui::icon_action(
                    "user-trash-symbolic",
                    fl!("activity-clear-history"),
                    (any && !self.clearing && self.pending.is_empty())
                        .then_some(Message::ConfirmClear)
                ),
            ]
            .spacing(8)
            .align_y(cosmic::iced::Alignment::Center),
            widget::text(fl!("activity-description")),
            widget::segmented_control::horizontal(&self.tabs).on_activate(Message::SelectTab),
        ]
        .spacing(ui::GAP)
        .width(Length::Fill);
        if tab == Tab::All {
            content = content.push(
                widget::row![
                    widget::text::caption(fl!("activity-filter-label")),
                    widget::dropdown(
                        Filter::ALL
                            .iter()
                            .map(|filter| filter.label())
                            .collect::<Vec<_>>(),
                        Filter::ALL.iter().position(|filter| *filter == self.filter),
                        |index| Message::SetFilter(Filter::ALL[index])
                    ),
                ]
                .spacing(12)
                .align_y(cosmic::iced::Alignment::Center),
            );
        }
        if !self.new.is_empty() {
            content = content.push(
                widget::button::suggested(format!(
                    "{} ({})",
                    fl!("activity-show-new"),
                    self.new.len()
                ))
                .on_press(Message::PublishNew),
            );
        }
        if self.confirm_clear {
            let mut confirmation = widget::column![
                widget::text::heading(fl!("activity-clear-title")),
                widget::text(fl!("activity-clear-description"))
            ]
            .spacing(8);
            confirmation = confirmation.push(
                widget::row![
                    widget::button::destructive(fl!("activity-clear-confirm"))
                        .on_press_maybe((!self.clearing).then_some(Message::ClearAll)),
                    widget::button::standard(fl!("cancel"))
                        .on_press_maybe((!self.clearing).then_some(Message::CancelClear))
                ]
                .spacing(8),
            );
            content = content.push(
                widget::container(confirmation)
                    .padding(16)
                    .width(Length::Fill)
                    .class(cosmic::style::Container::Card),
            );
        }
        if refreshing || (!self.has_loaded && self.error.is_none()) {
            content = content.push(
                widget::row![
                    widget::indeterminate_circular().size(20.),
                    widget::text::caption(if self.has_loaded {
                        fl!("activity-refreshing")
                    } else {
                        fl!("activity-loading")
                    })
                ]
                .spacing(8),
            );
        }
        if let Some(error) = &self.error {
            content = content.push(
                widget::column![
                    widget::text(error),
                    widget::button::standard(fl!("reading-retry"))
                        .on_press_maybe((!self.loading).then_some(Message::Retry))
                ]
                .spacing(8),
            );
        }
        let filter_notice = reading::filter_notice(cache);
        let waiting_for_filters = filter_notice.is_some();
        if let Some(notice) = filter_notice {
            content = content.push(notice.map(Message::Status));
        }
        let mut list = widget::column![].spacing(12);
        let mut date = None;
        for group in &groups {
            if date != Some(group.date) {
                date = Some(group.date);
                list = list.push(
                    widget::container(widget::text::caption_heading(model::day_label(group.date)))
                        .padding([12, 0]),
                );
            }
            list = list.push(view::notification(group, self, cache));
        }
        if groups.is_empty() && self.has_loaded && self.error.is_none() && !waiting_for_filters {
            let (title, description) = match tab {
                Tab::Requests => (
                    fl!("activity-no-requests"),
                    fl!("activity-no-requests-description"),
                ),
                Tab::Mentions => (
                    fl!("activity-no-mentions"),
                    fl!("activity-no-mentions-description"),
                ),
                Tab::All if self.filter != Filter::All || any => (
                    fl!("activity-no-matches"),
                    fl!("activity-no-matches-description"),
                ),
                _ => (fl!("activity-empty"), fl!("activity-empty-description")),
            };
            list = list.push(ui::empty("mail-read-symbolic", title, description));
            if tab != Tab::All || self.filter != Filter::All {
                list = list.push(
                    widget::button::standard(fl!("activity-show-all")).on_press(Message::ShowAll),
                );
            }
        }
        if self.loading && self.load_kind == LoadKind::Older {
            list = list.push(widget::indeterminate_circular().size(24.));
        } else if self.has_loaded && self.more {
            list = list.push(
                widget::button::standard(fl!("activity-load-older"))
                    .on_press_maybe((!self.loading && !self.clearing).then_some(Message::LoadMore)),
            );
        }
        content = content.push(list);
        widget::scrollable(
            widget::container(
                widget::container(content)
                    .max_width(ui::CONTENT_WIDTH)
                    .padding(16),
            )
            .center_x(Length::Fill),
        )
        .height(Length::Fill)
        .width(Length::Fill)
        .into()
    }
    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        let mut tasks = vec![];
        match message {
            Message::SetClient(client) => {
                if self.mastodon != client {
                    self.reset(client);
                    return self.load_cached();
                }
                self.publish_new();
            }
            Message::PrependNotification(notification) => {
                if self.dismissed.contains(&notification.id) {
                    return Task::none();
                }
                if !self.notifications.contains(&notification.id)
                    && !self.new.contains(&notification.id)
                {
                    self.new.push_front(notification.id.clone());
                }
                tasks.push(cache_notification(notification));
            }
            Message::Refresh if !self.loading && !self.clearing => {
                self.start_load(LoadKind::Refresh)
            }
            Message::Retry if self.confirm_clear && !self.clearing => {
                return self.update(Message::ClearAll);
            }
            Message::Retry if !self.loading && !self.clearing => self.start_load(self.load_kind),
            Message::LoadMore if !self.loading && self.more && !self.clearing => {
                self.start_load(LoadKind::Older)
            }
            Message::PageLoaded(request, result) if request == self.request => {
                self.loading = false;
                self.has_loaded = true;
                match result {
                    Ok(page) => {
                        self.error = None;
                        let keep_cursor = self.load_kind == LoadKind::Refresh
                            && self.max_id.is_some()
                            && !page
                                .notifications
                                .iter()
                                .any(|notification| Some(&notification.id) == self.max_id.as_ref());
                        if self.load_kind == LoadKind::Refresh && page.notifications.is_empty() {
                            let ids = self.notifications.iter().cloned().collect::<Vec<_>>();
                            self.remove(&ids);
                            self.max_id = None;
                            self.more = false;
                            tasks.push(cosmic::task::message(app::Message::RemoveNotifications(
                                ids,
                            )));
                        } else if !keep_cursor {
                            self.max_id = page.cursor;
                            self.more = page.more;
                        }
                        for notification in page.notifications {
                            if self.dismissed.contains(&notification.id) {
                                continue;
                            }
                            if !self.notifications.contains(&notification.id) {
                                self.notifications.push_back(notification.id.clone());
                            }
                            self.new.retain(|id| id != &notification.id);
                            tasks.push(cache_notification(notification));
                        }
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::SelectTab(entity) => {
                self.tabs.activate(entity);
                self.filter = Filter::All;
            }
            Message::SetFilter(filter) => self.filter = filter,
            Message::ShowAll => {
                self.tabs.activate_position(0);
                self.filter = Filter::All;
            }
            Message::PublishNew => self.publish_new(),
            Message::TogglePeople(key) => {
                if !self.expanded.remove(&key) {
                    self.expanded.insert(key);
                }
            }
            Message::ConfirmClear => self.confirm_clear = true,
            Message::CancelClear if !self.clearing => self.confirm_clear = false,
            Message::ClearAll
                if self.confirm_clear && !self.clearing && self.pending.is_empty() =>
            {
                self.clearing = true;
                // Cancel an older page before taking the clear snapshot so it
                // cannot reintroduce server history while clearing is pending.
                self.request += 1;
                self.loading = false;
                self.has_loaded = true;
                let ids = self
                    .notifications
                    .iter()
                    .chain(&self.new)
                    .cloned()
                    .collect();
                let client = self.mastodon.clone();
                tasks.push(cosmic::task::future(async move {
                    let result = crate::client::within_timeout(
                        client.dismiss_notifications(),
                        fl!("activity-clear-error"),
                    )
                    .await
                    .map(|_| ());
                    app::Message::Notifications(Message::Cleared(ids, result))
                }));
            }
            Message::Cleared(ids, result) => {
                self.clearing = false;
                match result {
                    Ok(()) => {
                        self.remove(&ids);
                        self.confirm_clear = false;
                        self.max_id = None;
                        self.more = false;
                        self.error = None;
                        tasks.push(cosmic::task::message(app::Message::RemoveNotifications(
                            ids,
                        )));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::Dismiss(ids) if !self.clearing => {
                let ids: Vec<_> = ids
                    .into_iter()
                    .filter(|id| self.pending.insert(id.clone()))
                    .collect();
                for id in &ids {
                    self.action_errors.remove(id);
                }
                let client = self.mastodon.clone();
                tasks.push(cosmic::task::future(async move {
                    let results = futures_util::future::join_all(ids.into_iter().map(|id| {
                        let client = client.clone();
                        async move {
                            let result = crate::client::within_timeout(
                                client.dismiss_notification(id.clone()),
                                fl!("activity-dismiss-error"),
                            )
                            .await
                            .map(|_| ());
                            (id, result)
                        }
                    }))
                    .await;
                    app::Message::Notifications(Message::Dismissed(results))
                }));
            }
            Message::Dismissed(results) => {
                let mut removed = vec![];
                for (id, result) in results {
                    self.pending.remove(&id);
                    match result {
                        Ok(()) => removed.push(id),
                        Err(error) => {
                            self.action_errors.insert(id, error);
                        }
                    }
                }
                self.remove(&removed);
                tasks.push(cosmic::task::message(app::Message::RemoveNotifications(
                    removed,
                )));
            }
            Message::FollowDecision(id, account, accept) if !self.clearing => {
                if !self.pending.insert(id.clone()) {
                    return Task::none();
                }
                self.action_errors.remove(&id);
                let client = self.mastodon.clone();
                tasks.push(cosmic::task::future(async move {
                    let request = async {
                        if accept {
                            client.accept_follow_request(account).await
                        } else {
                            client.reject_follow_request(account).await
                        }
                    };
                    let result =
                        crate::client::within_timeout(request, fl!("activity-request-error"))
                            .await
                            .map(|response| response.json);
                    app::Message::Notifications(Message::FollowFinished(id, result))
                }));
            }
            Message::FollowFinished(id, result) => {
                self.pending.remove(&id);
                match result {
                    Ok(relationship) => {
                        self.remove(std::slice::from_ref(&id));
                        tasks.push(cosmic::task::message(app::Message::RemoveNotifications(
                            vec![id],
                        )));
                        tasks.push(cosmic::task::message(app::Message::CacheRelationship(
                            relationship,
                        )));
                    }
                    Err(error) => {
                        self.action_errors.insert(id, error);
                    }
                }
            }
            Message::Status(message) => return status::update(message),
            _ => (),
        }
        Task::batch(tasks)
    }
    pub fn subscription(&self) -> Subscription<Message> {
        if !self.is_authenticated() {
            return Subscription::none();
        }
        if self.loading || (!self.has_loaded && self.error.is_none()) {
            let cursor = if self.load_kind == LoadKind::Older {
                self.max_id.clone()
            } else {
                None
            };
            return fetch::timeline(self.mastodon.clone(), self.request, cursor);
        }
        Subscription::none()
    }
}
