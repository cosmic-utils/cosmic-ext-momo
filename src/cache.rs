use std::collections::{HashMap, HashSet};

use cosmic::{
    iced::core::image,
    widget::{self, image::Handle},
};
use megalodon::entities::{Account, Notification, Relationship, Status};

use crate::config::FeedDensity;
use crate::error::Error;

pub const MAX_RETAINED_ITEMS: usize = 500;
pub const MAX_BUFFERED_ITEMS: usize = 100;

#[derive(Debug, Clone)]
pub struct Cache {
    pub deleted: HashSet<String>,
    pub reading: crate::features::reading::State,
    pub handles: HashMap<String, Handle>,
    pub statuses: HashMap<String, Status>,
    pub notifications: HashMap<String, Notification>,
    /// The authenticated user's relationship (following/muting/blocking) to
    /// each account whose profile has been viewed, keyed by account id.
    pub relationships: HashMap<String, Relationship>,
    /// The currently authenticated account, if logged in. Used to decide
    /// which statuses/relationships belong to the current user (e.g. to
    /// show a delete action only on your own posts).
    pub me: Option<Account>,
    /// Timeline display preferences from [`crate::config::MomoConfig`],
    /// snapshotted here since it's already threaded through every feature's
    /// `view(&Cache)` call.
    pub hide_boosts: bool,
    pub hide_replies: bool,
    pub feed_density: FeedDensity,
    /// Set whenever new content is cached; cleared once flushed to disk.
    /// Lets the periodic save subscription skip writing when nothing changed.
    pub dirty: bool,
}

impl Cache {
    pub fn new() -> Self {
        Self {
            deleted: HashSet::new(),
            reading: crate::features::reading::State::new(false),
            handles: HashMap::new(),
            statuses: HashMap::new(),
            notifications: HashMap::new(),
            relationships: HashMap::new(),
            me: None,
            hide_boosts: false,
            hide_replies: false,
            feed_density: FeedDensity::default(),
            dirty: false,
        }
    }

    /// Whether a status should be shown given the current display preferences.
    pub fn is_visible(&self, status: &Status) -> bool {
        !(self.hide_boosts && status.reblog.is_some()
            || self.hide_replies && status.in_reply_to_id.is_some())
    }

    pub fn insert_relationship(&mut self, relationship: Relationship) {
        self.relationships
            .insert(relationship.id.clone(), relationship);
    }

    /// Whether the given account id belongs to the authenticated user.
    pub fn is_me(&self, account_id: &str) -> bool {
        self.me
            .as_ref()
            .is_some_and(|account| account.id == account_id)
    }

    pub fn insert_status(&mut self, mut status: Status) {
        if self.deleted.contains(&status.id)
            || status
                .reblog
                .as_ref()
                .is_some_and(|post| self.deleted.contains(&post.id))
        {
            return;
        }
        if self
            .statuses
            .get(&status.id)
            .is_some_and(|old| old.edited_at > status.edited_at)
        {
            return;
        }
        if let Some(reblog) = &mut status.reblog {
            self.insert_status((**reblog).clone());
            if let Some(current) = self.statuses.get(&reblog.id) {
                **reblog = current.clone();
            }
        }
        // New IDs cannot have embedded copies: boosts insert their originals first.
        if self.statuses.contains_key(&status.id) {
            for cached in self.statuses.values_mut() {
                if let Some(reblog) = &mut cached.reblog {
                    if reblog.id == status.id {
                        **reblog = status.clone();
                    }
                }
            }
            for notification in self.notifications.values_mut() {
                if let Some(post) = &mut notification.status {
                    if post.id == status.id {
                        *post = status.clone();
                    } else if let Some(reblog) = &mut post.reblog {
                        if reblog.id == status.id {
                            **reblog = status.clone();
                        }
                    }
                }
            }
        }
        self.statuses.insert(status.id.clone(), status);
        self.dirty = true;
    }

    /// Tombstones prevent late REST responses from resurrecting streamed deletions.
    /// Return wrapper IDs as well so every feed can drop boosts of the deleted post.
    pub fn delete_status(&mut self, id: &str) -> Vec<String> {
        let mut ids = vec![id.to_owned()];
        ids.extend(
            self.statuses
                .values()
                .filter(|post| post.reblog.as_ref().is_some_and(|post| post.id == id))
                .map(|post| post.id.clone()),
        );
        self.deleted.extend(ids.iter().cloned());
        self.statuses.retain(|id, _| !self.deleted.contains(id));
        self.notifications.retain(|_, notification| {
            !notification.status.as_ref().is_some_and(|post| {
                self.deleted.contains(&post.id)
                    || post
                        .reblog
                        .as_ref()
                        .is_some_and(|post| self.deleted.contains(&post.id))
            })
        });
        self.dirty = true;
        ids
    }

    pub fn insert_notification(&mut self, notification: Notification) {
        if notification.status.as_ref().is_some_and(|post| {
            self.deleted.contains(&post.id)
                || post
                    .reblog
                    .as_ref()
                    .is_some_and(|post| self.deleted.contains(&post.id))
        }) {
            return;
        }
        let mut notification = notification;
        if let Some(status) = &notification.status {
            self.insert_status(status.clone());
        }
        if let Some(status) = &mut notification.status {
            if let Some(current) = self.statuses.get(&status.id) {
                *status = current.clone();
            }
        }
        self.notifications
            .insert(notification.id.to_string(), notification);
        self.dirty = true;
    }

    pub fn insert_handle(&mut self, url: String, handle: Handle) {
        self.handles.insert(url, handle);
    }

    pub fn retain_referenced(
        &mut self,
        mut statuses: HashSet<String>,
        notifications: &HashSet<String>,
        mut images: HashSet<String>,
        mut accounts: HashSet<String>,
    ) -> HashSet<String> {
        self.notifications
            .retain(|id, _| notifications.contains(id));
        for notification in self.notifications.values() {
            if let Some(post) = &notification.status {
                statuses.insert(post.id.clone());
            }
            if let Some(account) = &notification.account {
                accounts.insert(account.id.clone());
            }
            images.extend(extract_notification_images(notification));
        }
        let mut pending: Vec<_> = statuses.iter().cloned().collect();
        while let Some(id) = pending.pop() {
            if let Some(post) = self.statuses.get(&id) {
                if let Some(original) = &post.reblog {
                    if statuses.insert(original.id.clone()) {
                        pending.push(original.id.clone());
                    }
                }
            }
        }
        self.statuses.retain(|id, _| statuses.contains(id));
        let mut polls = HashSet::new();
        for post in self.statuses.values() {
            images.extend(extract_status_images(post));
            accounts.insert(post.account.id.clone());
            if let Some(poll) = &post.poll {
                polls.insert(poll.id.clone());
            }
        }
        self.handles.retain(|url, _| images.contains(url));
        self.relationships.retain(|id, _| accounts.contains(id));
        self.reading.warnings.retain(|id| statuses.contains(id));
        self.reading.media.retain(|id| statuses.contains(id));
        self.reading
            .collapsed_replies
            .retain(|id| statuses.contains(id));
        self.reading
            .filter_reveals
            .retain(|(id, _)| statuses.contains(id));
        self.reading.polls.retain(|id, _| polls.contains(id));
        self.reading.choices.retain(|id, _| polls.contains(id));
        self.reading.poll_errors.retain(|id, _| polls.contains(id));
        images
    }

    pub fn clear(&mut self) {
        self.deleted.clear();
        self.reading = crate::features::reading::State::new(false);
        self.statuses.clear();
        self.notifications.clear();
        self.handles.clear();
        self.relationships.clear();
        self.me = None;
        self.dirty = false;
    }
}

pub fn fallback_avatar<'a>() -> widget::Image<'a> {
    widget::image(image::Handle::from_bytes(
        include_bytes!("../assets/missing.png").to_vec(),
    ))
}

pub fn fallback_handle() -> widget::image::Handle {
    image::Handle::from_bytes(include_bytes!("../assets/missing.png").to_vec())
}

pub async fn get(url: impl ToString, epoch: u64) -> Result<Handle, Error> {
    let url = url.to_string();

    if let Some(bytes) = load_cached_image(url.clone()).await {
        return Ok(Handle::from_bytes(bytes));
    }

    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?
        .get(&url)
        .send()
        .await?;
    match response.error_for_status() {
        Ok(response) => {
            if response
                .content_length()
                .is_some_and(|length| length > crate::persistence::MAX_IMAGE_BYTES)
            {
                return Err(Error::ImageTooLarge);
            }
            let mut response = response;
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if bytes.len().saturating_add(chunk.len()) as u64
                    > crate::persistence::MAX_IMAGE_BYTES
                {
                    return Err(Error::ImageTooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            save_cached_image(url, bytes.clone(), epoch);
            Ok(Handle::from_bytes(bytes))
        }
        Err(err) => Err(err.into()),
    }
}

/// Disk reads/writes are blocking; run them on tokio's blocking pool so a
/// cold or growing image cache doesn't stall the async executor (and with
/// it, every other in-flight fetch) one file at a time.
async fn load_cached_image(url: String) -> Option<Vec<u8>> {
    tokio::task::spawn_blocking(move || crate::persistence::load_image(&url))
        .await
        .ok()
        .flatten()
}

fn save_cached_image(url: String, bytes: Vec<u8>, epoch: u64) {
    tokio::task::spawn_blocking(move || crate::persistence::save_image(&url, &bytes, epoch));
}

pub fn extract_status_images(status: &Status) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    if !status.account.avatar.is_empty() {
        urls.push(status.account.avatar.clone());
    }
    if !status.account.header.is_empty() {
        urls.push(status.account.header.clone());
    }

    if let Some(reblog) = &status.reblog {
        if !reblog.account.avatar.is_empty() {
            urls.push(reblog.account.avatar.clone());
        }
        if !reblog.account.header.is_empty() {
            urls.push(reblog.account.header.clone());
        }
        if let Some(card) = &reblog.card {
            if let Some(image) = &card.image {
                urls.push(image.clone());
            }
        }
        for attachment in &reblog.media_attachments {
            if let Some(url) = &attachment.preview_url {
                urls.push(url.clone());
            }
        }
    }

    if let Some(card) = &status.card {
        if let Some(image) = &card.image {
            urls.push(image.clone());
        }
    }

    for attachment in &status.media_attachments {
        if let Some(url) = &attachment.preview_url {
            urls.push(url.clone());
        }
    }

    urls
}

pub fn extract_notification_images(notification: &Notification) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    if let Some(account) = &notification.account {
        if !account.avatar.is_empty() {
            urls.push(account.avatar.clone());
        }
        if !account.header.is_empty() {
            urls.push(account.header.clone());
        }
    }

    if let Some(status) = &notification.status {
        if !status.account.avatar.is_empty() {
            urls.push(status.account.avatar.clone());
        }
        if !status.account.header.is_empty() {
            urls.push(status.account.header.clone());
        }
        if let Some(card) = &status.card {
            if let Some(image) = &card.image {
                urls.push(image.clone());
            }
        }
        for attachment in &status.media_attachments {
            if let Some(url) = &attachment.preview_url {
                urls.push(url.clone());
            }
        }
    }
    urls
}
