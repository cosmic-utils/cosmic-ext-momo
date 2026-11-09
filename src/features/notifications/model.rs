//! Presentation groups do not replace notification IDs: every member remains
//! individually addressable for dismissal, caching, and streaming updates.
use crate::{
    cache::Cache,
    features::reading::filters::{Context, Decision},
    fl,
};
use chrono::{DateTime, Local, NaiveDate, Utc};
use megalodon::entities::{notification::NotificationType as Kind, Account, Notification, Status};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    All,
    Mentions,
    Requests,
}
impl Tab {
    pub fn matches(self, kind: &Kind) -> bool {
        match self {
            Self::All => true,
            Self::Mentions => matches!(kind, Kind::Mention | Kind::Quote),
            Self::Requests => *kind == Kind::FollowRequest,
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Favorites,
    Boosts,
    Follows,
    Updates,
    Other,
}
impl Filter {
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::Favorites,
        Self::Boosts,
        Self::Follows,
        Self::Updates,
        Self::Other,
    ];
    pub fn label(self) -> String {
        match self {
            Self::All => fl!("activity-all-types"),
            Self::Favorites => fl!("activity-favorites"),
            Self::Boosts => fl!("activity-boosts"),
            Self::Follows => fl!("activity-follows"),
            Self::Updates => fl!("activity-updates"),
            Self::Other => fl!("activity-other"),
        }
    }
    pub fn matches(self, kind: &Kind) -> bool {
        match self {
            Self::All => true,
            Self::Favorites => *kind == Kind::Favourite,
            Self::Boosts => *kind == Kind::Reblog,
            Self::Follows => matches!(kind, Kind::Follow | Kind::FollowRequest),
            Self::Updates => matches!(
                kind,
                Kind::Update
                    | Kind::Status
                    | Kind::PollVote
                    | Kind::PollExpired
                    | Kind::QuotedUpdate
            ),
            Self::Other => !matches!(
                kind,
                Kind::Mention
                    | Kind::Favourite
                    | Kind::Reblog
                    | Kind::Follow
                    | Kind::FollowRequest
                    | Kind::Update
                    | Kind::Status
                    | Kind::PollVote
                    | Kind::PollExpired
                    | Kind::Quote
                    | Kind::QuotedUpdate
            ),
        }
    }
}
#[derive(Debug, Clone)]
pub struct Group {
    pub key: String,
    pub ids: Vec<String>,
    pub actors: Vec<Account>,
    pub kind: Kind,
    pub created_at: DateTime<Utc>,
    pub date: NaiveDate,
    pub post: Option<Status>,
}
pub fn visible(notification: &Notification, cache: &Cache) -> bool {
    notification.status.as_ref().is_none_or(|post| {
        let post = cache.statuses.get(&post.id).unwrap_or(post);
        !matches!(
            cache.reading.filters.decision(post, Context::Notifications),
            Decision::Hide | Decision::Loading | Decision::Failed
        )
    })
}
pub fn groups<'a>(
    notifications: impl Iterator<Item = &'a Notification>,
    cache: &Cache,
    tab: Tab,
    filter: Filter,
) -> Vec<Group> {
    let mut groups: Vec<Group> = vec![];
    let mut positions = HashMap::new();
    for notification in notifications.filter(|notification| {
        tab.matches(&notification.r#type)
            && filter.matches(&notification.r#type)
            && visible(notification, cache)
    }) {
        let date = notification.created_at.with_timezone(&Local).date_naive();
        let aggregate = matches!(notification.r#type, Kind::Favourite | Kind::Reblog);
        let key = if aggregate {
            notification
                .status
                .as_ref()
                .map(|post| format!("{}:{}:{date}", notification.r#type, post.id))
                .unwrap_or_else(|| notification.id.clone())
        } else {
            notification.id.clone()
        };
        if let Some(index) = positions.get(&key).copied() {
            let group: &mut Group = &mut groups[index];
            group.ids.push(notification.id.clone());
            if let Some(account) = &notification.account {
                if !group.actors.iter().any(|actor| actor.id == account.id) {
                    group.actors.push(account.clone());
                }
            }
            if notification.created_at > group.created_at {
                group.created_at = notification.created_at;
            }
        } else {
            positions.insert(key.clone(), groups.len());
            groups.push(Group {
                key,
                ids: vec![notification.id.clone()],
                actors: notification.account.iter().cloned().collect(),
                kind: notification.r#type.clone(),
                created_at: notification.created_at,
                date,
                post: notification
                    .status
                    .as_ref()
                    .map(|post| cache.statuses.get(&post.id).unwrap_or(post).clone()),
            });
        }
    }
    groups.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.key.cmp(&a.key))
    });
    groups
}
pub fn actor_name(account: &Account) -> String {
    if account.display_name.trim().is_empty() {
        account.acct.clone()
    } else {
        account.display_name.clone()
    }
}
pub fn day_label(day: NaiveDate) -> String {
    let today = Local::now().date_naive();
    if day == today {
        fl!("activity-today")
    } else if Some(day) == today.pred_opt() {
        fl!("activity-yesterday")
    } else {
        day.format("%b %-d, %Y").to_string()
    }
}
