//! Feed-shaped timelines: Home, Explore (public), Local, and Federated.
//!
//! All four share the same fetch/paginate/render shape, so they're modeled as
//! one [`Timeline`] parameterized by [`TimelineKind`] instead of four
//! near-identical structs.

pub mod fetch;

use std::collections::{HashSet, VecDeque};

use cosmic::{
    app::Task,
    iced::widget::scrollable::{Direction, Scrollbar},
    iced::{Length, Subscription},
    widget, Element,
};
use megalodon::entities::Status;

use crate::{
    app,
    cache::{self, Cache},
    client::Client,
    features::status::{self, StatusOptions},
    fl, ui,
};

/// Which Mastodon timeline a [`Timeline`] instance renders.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TimelineKind {
    /// The authenticated user's home feed.
    Home,
    /// The public (federated) timeline, unfiltered.
    Public,
    /// The local instance's public timeline.
    Local,
    /// The public timeline, filtered down to posts from remote instances.
    Federated,
    /// Statuses the user has favourited.
    Favorites,
    /// Statuses the user has bookmarked.
    Bookmarks,
    /// A hashtag's timeline.
    Tag(String),
    /// A user list's timeline.
    List(String),
}

impl TimelineKind {
    /// A filesystem-safe identifier used as the disk cache filename stem.
    pub fn slug(&self) -> String {
        match self {
            TimelineKind::Home => "home".to_string(),
            TimelineKind::Public => "public".to_string(),
            TimelineKind::Local => "local".to_string(),
            TimelineKind::Federated => "federated".to_string(),
            TimelineKind::Favorites => "favorites".to_string(),
            TimelineKind::Bookmarks => "bookmarks".to_string(),
            TimelineKind::Tag(name) => scoped_slug("tag", name),
            TimelineKind::List(id) => scoped_slug("list", id),
        }
    }
}

fn scoped_slug(prefix: &str, value: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hash);
    format!("{prefix}-{:016x}", hash.finish())
}

/// State for a single feed-shaped timeline.
#[derive(Debug, Clone)]
pub struct Timeline {
    pub mastodon: Client,
    kind: TimelineKind,
    display_title: Option<String>,
    statuses: VecDeque<String>,
    max_id: Option<String>,
    /// Whether a page-load (initial or "load more") fetch is in flight.
    loading: bool,
    /// Whether at least one fetch has completed, so an empty `statuses` can
    /// be told apart from "hasn't tried yet" (still loading).
    has_loaded: bool,
    request: u64,
    older: bool,
    more: bool,
    error: Option<String>,
    incoming: VecDeque<String>,
    incoming_overflow: bool,
    removed: HashSet<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    SetClient(Client),
    Scrolled(cosmic::iced::widget::scrollable::Viewport),
    Refresh,
    Recover,
    Retry,
    PublishNew,
    IncomingStatus(Status),
    PageLoaded(u64, Result<fetch::Page, String>),
    PrependStatus(Status),
    CollectionChanged(Status),
    DeleteStatus(String),
    Status(status::Message),
    LoadMore,
}

impl Timeline {
    pub fn new(mastodon: Client, kind: TimelineKind) -> Self {
        Self {
            mastodon,
            kind,
            display_title: None,
            statuses: VecDeque::new(),
            max_id: None,
            loading: false,
            has_loaded: false,
            request: 0,
            older: false,
            more: true,
            error: None,
            incoming: VecDeque::new(),
            incoming_overflow: false,
            removed: HashSet::new(),
        }
    }

    pub fn with_title(mut self, title: String) -> Self {
        self.display_title = Some(title);
        self
    }

    pub fn is_authenticated(&self) -> bool {
        self.mastodon.is_authenticated()
    }

    /// Switch to a different account's client and drop this feed's current
    /// content, so a stale account's posts don't linger after switching.
    /// Call [`Timeline::load_cached`] afterward to repopulate from disk.
    pub fn reset(&mut self, mastodon: Client) {
        let request = self.request + 1;
        let title = self.display_title.clone();
        *self = Self::new(mastodon, self.kind.clone());
        self.display_title = title;
        self.request = request;
    }

    /// Load this feed's last-saved snapshot from disk (if any) so the view
    /// has something to render immediately, before the network fetch lands.
    pub fn load_cached(&mut self) -> Task<app::Message> {
        if self.mastodon.is_authenticated() && self.mastodon.account_id.is_empty() {
            return Task::none();
        }
        let cached: Vec<Status> =
            crate::persistence::load_snapshot(&self.mastodon.cache_key(), &self.kind.slug());
        let mut tasks = vec![];
        for status in cached {
            if !self.statuses.contains(&status.id) {
                self.statuses.push_back(status.id.clone());
            }
            tasks.push(cache_status(status));
        }
        Task::batch(tasks)
    }

    pub fn status_ids(&self) -> impl Iterator<Item = &String> {
        self.statuses.iter().chain(&self.incoming)
    }

    pub fn snapshot(&self, cache: &Cache) -> Option<crate::persistence::Snapshot> {
        if self.mastodon.is_authenticated() && self.mastodon.account_id.is_empty() {
            return None;
        }
        let statuses = self
            .incoming
            .iter()
            .chain(&self.statuses)
            .filter_map(|id| cache.statuses.get(id))
            .take(crate::persistence::MAX_STATUSES)
            .cloned()
            .collect();
        Some(crate::persistence::Snapshot::Statuses(
            self.kind.slug(),
            statuses,
        ))
    }

    fn title(&self) -> String {
        if let Some(title) = &self.display_title {
            return title.clone();
        }
        match &self.kind {
            TimelineKind::Home => fl!("home"),
            TimelineKind::Public => fl!("explore"),
            TimelineKind::Local => fl!("local"),
            TimelineKind::Federated => fl!("federated"),
            TimelineKind::Favorites => fl!("favorites"),
            TimelineKind::Bookmarks => fl!("bookmarks"),
            TimelineKind::Tag(tag) => format!("#{tag}"),
            TimelineKind::List(_) => fl!("lists"),
        }
    }
    pub fn view<'a>(&'a self, cache: &'a Cache) -> Element<'a, Message> {
        let header = widget::row![
            widget::text::title2(self.title()),
            widget::space::horizontal(),
        ]
        .push_maybe((!self.incoming.is_empty()).then(|| {
            widget::button::suggested(format!(
                "{} ({})",
                fl!("feed-show-new"),
                self.incoming.len()
            ))
            .on_press(Message::PublishNew)
        }))
        .push(ui::refresh_action(
            self.loading && !self.older,
            fl!("feed-refresh"),
            (!self.loading).then_some(Message::Refresh),
        ))
        .spacing(8)
        .align_y(cosmic::iced::Alignment::Center);
        let mut content = widget::column![header].spacing(ui::GAP).width(Length::Fill);
        if let Some(error) = &self.error {
            content = content.push(
                widget::column![
                    widget::text(error),
                    widget::button::standard(fl!("reading-retry")).on_press(Message::Retry)
                ]
                .spacing(8),
            );
        }
        if let Some(notice) = crate::features::reading::filter_notice(cache) {
            content = content.push(notice.map(Message::Status));
        } else {
            let context = match self.kind {
                TimelineKind::Public
                | TimelineKind::Local
                | TimelineKind::Federated
                | TimelineKind::Tag(_) => crate::features::reading::filters::Context::Public,
                _ => crate::features::reading::filters::Context::Home,
            };
            let mut visible = 0;
            for post in self
                .statuses
                .iter()
                .filter_map(|id| cache.statuses.get(id))
                .filter(|post| cache.is_visible(post))
            {
                if cache.reading.filters.decision(post, context)
                    == crate::features::reading::filters::Decision::Hide
                {
                    continue;
                }
                visible += 1;
                content = content.push(
                    widget::container(
                        status::status(post, StatusOptions::all().context(context), cache)
                            .map(Message::Status),
                    )
                    .width(Length::Fill)
                    .class(cosmic::style::Container::Card),
                );
            }
            if visible == 0 && self.has_loaded && self.error.is_none() {
                content = content.push(ui::empty(
                    "view-list-symbolic",
                    fl!("reading-no-visible-posts"),
                    fl!("feed-empty-description"),
                ));
            }
        }
        if self.loading || (!self.has_loaded && self.error.is_none()) {
            content = content.push(
                widget::row![
                    widget::indeterminate_circular().size(20.),
                    widget::text::caption(fl!("feed-loading"))
                ]
                .spacing(8),
            );
        } else if self.has_loaded && self.more && self.error.is_none() {
            content = content.push(
                widget::button::standard(fl!("reading-load-more")).on_press(Message::LoadMore),
            );
        } else if self.has_loaded && self.error.is_none() && !self.statuses.is_empty() {
            content = content.push(widget::text::caption(fl!("feed-end")));
        }
        widget::scrollable(
            widget::container(
                widget::container(content)
                    .max_width(ui::CONTENT_WIDTH)
                    .padding(16),
            )
            .center_x(Length::Fill),
        )
        .id(cosmic::iced::core::widget::Id::new("main-scroll"))
        .on_scroll(Message::Scrolled)
        .direction(Direction::Vertical(Scrollbar::default()))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn scroll_id(&self) -> cosmic::iced::core::widget::Id {
        cosmic::iced::core::widget::Id::new(format!(
            "feed:{}:{}",
            self.mastodon.cache_key(),
            self.kind.slug()
        ))
    }
    fn top(&self) -> Task<app::Message> {
        cosmic::iced::widget::scrollable::snap_to(
            cosmic::iced::core::widget::Id::new("main-scroll"),
            cosmic::iced::widget::scrollable::RelativeOffset {
                x: Some(0.0),
                y: Some(0.0),
            },
        )
    }
    fn begin_load(&mut self, older: bool) {
        self.request += 1;
        self.older = older;
        self.loading = true;
        self.error = None;
    }

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        let mut tasks = vec![];
        let paging_older = matches!(&message, Message::PageLoaded(..)) && self.older;
        match message {
            Message::SetClient(mastodon) => self.mastodon = mastodon,
            Message::Scrolled(offset) => tasks.push(cosmic::task::message(
                app::Message::FeedScrolled(self.scroll_id(), offset),
            )),
            Message::Refresh if !self.loading => self.begin_load(false),
            Message::Recover => self.begin_load(false),
            Message::Retry if !self.loading => self.begin_load(self.older),
            Message::LoadMore if !self.loading && self.more => self.begin_load(true),
            Message::PublishNew => {
                tasks.push(self.top());
                while let Some(id) = self.incoming.pop_back() {
                    if !self.statuses.contains(&id) {
                        self.statuses.push_front(id);
                    }
                }
                if self.incoming_overflow {
                    self.incoming_overflow = false;
                    self.begin_load(false);
                }
            }
            Message::IncomingStatus(post) => {
                if self.removed.contains(&post.id) {
                    return Task::none();
                }
                if !self.statuses.contains(&post.id) && !self.incoming.contains(&post.id) {
                    self.incoming.push_front(post.id.clone());
                }
                tasks.push(cache_status(post));
            }
            Message::PageLoaded(request, result) if request == self.request => {
                self.loading = false;
                self.has_loaded = true;
                match result {
                    Err(error) => self.error = Some(error),
                    Ok(page) => {
                        self.error = None;
                        // A refresh replaces the page sequence so older paging cannot skip a gap in new posts.
                        if !self.older {
                            self.statuses.clear();
                            tasks.push(self.top());
                        }
                        self.more = page.more && (page.cursor != self.max_id || !self.older);
                        self.max_id = page.cursor;
                        for post in page.statuses {
                            if self.removed.contains(&post.id) {
                                continue;
                            }
                            self.incoming.retain(|id| id != &post.id);
                            if !self.statuses.contains(&post.id) {
                                self.statuses.push_back(post.id.clone());
                            }
                            tasks.push(cache_status(post));
                        }
                    }
                }
            }
            Message::CollectionChanged(status) => {
                let included = match self.kind {
                    TimelineKind::Favorites => status.favourited.unwrap_or(false),
                    TimelineKind::Bookmarks => status.bookmarked.unwrap_or(false),
                    _ => return Task::none(),
                };
                if !included {
                    self.statuses.retain(|id| id != &status.id);
                } else if !self.statuses.contains(&status.id) {
                    self.statuses.push_front(status.id.clone());
                }
                self.incoming.retain(|id| id != &status.id);
                // Mutation responses already contain the confirmed status. An immediate
                // collection reload can undo them: Mastodon removes favourites in a worker.
                // Cancel older pages too, so they cannot restore the pre-mutation state.
                self.request += 1;
                self.loading = false;
                self.has_loaded = true;
                self.error = None;
            }
            Message::PrependStatus(status) => {
                self.incoming.retain(|id| id != &status.id);
                if !self.statuses.contains(&status.id) {
                    self.statuses.push_front(status.id.clone());
                }
                tasks.push(cosmic::task::message(app::Message::CacheStatus(status)));
            }
            Message::DeleteStatus(id) => {
                self.removed.insert(id.clone());
                self.statuses.retain(|post| post != &id);
                self.incoming.retain(|post| post != &id);
            }
            Message::Status(message) => tasks.push(status::update(message)),
            _ => (),
        }
        let excess = self
            .statuses
            .len()
            .saturating_sub(cache::MAX_RETAINED_ITEMS);
        if paging_older {
            self.statuses.drain(..excess);
        } else {
            self.statuses.truncate(cache::MAX_RETAINED_ITEMS);
        }
        self.incoming_overflow |= self.incoming.len() > cache::MAX_BUFFERED_ITEMS;
        self.incoming.truncate(cache::MAX_BUFFERED_ITEMS);
        Task::batch(tasks)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let requires_auth = matches!(
            self.kind,
            TimelineKind::Home | TimelineKind::Favorites | TimelineKind::Bookmarks
        );
        if requires_auth && !self.is_authenticated() {
            return Subscription::none();
        }

        // `!self.has_loaded`, not `self.statuses.is_empty()`: a cache-preloaded
        // feed already has statuses to show, but still needs its first real
        // network fetch to refresh in the background — checking emptiness
        // here would skip that fetch entirely until the user scrolls.
        if (!self.has_loaded && self.error.is_none()) || self.loading {
            fetch::timeline(
                self.mastodon.clone(),
                self.kind.clone(),
                self.max_id.clone().filter(|_| self.older),
                self.request,
            )
        } else {
            Subscription::none()
        }
    }
}

/// Queue a status's images for download and store it in the shared cache.
fn cache_status(status: Status) -> Task<app::Message> {
    Task::batch([
        cosmic::task::message(app::Message::Fetch(cache::extract_status_images(&status))),
        cosmic::task::message(app::Message::CacheStatus(status)),
    ])
}

#[cfg(test)]
mod tests;
