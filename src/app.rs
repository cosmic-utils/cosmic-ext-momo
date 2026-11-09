// SPDX-License-Identifier: {{LICENSE}}

use crate::cache::Cache;
use crate::client::{Client, Session, Sessions};
use crate::config::TootConfig;
use crate::features::compose;
use crate::features::timeline::{Timeline, TimelineKind};
use crate::features::{
    accounts, hashtags, lists, notifications, onboarding, reading, search, settings, status,
    timeline,
};
use crate::fl;
use cosmic::app::{context_drawer, Core, Task};
use cosmic::cosmic_config;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Length, Subscription};
use cosmic::widget::about::About;
use cosmic::widget::image::Handle;
use cosmic::widget::menu::{ItemHeight, ItemWidth};
use cosmic::widget::toaster::{Toast, ToastId, Toasts};
use cosmic::widget::{self, menu, nav_bar};
use cosmic::{Application, ApplicationExt, Apply, Element};
use megalodon::entities::{Account, Notification, Status};

use std::collections::{HashMap, HashSet, VecDeque};

/// How many image downloads run at once. Keeping this bounded (rather than
/// firing every queued fetch concurrently) means the front of the queue —
/// newest posts' images, since they're enqueued first — actually gets a
/// worker slot promptly instead of competing with everything else queued.
const MAX_CONCURRENT_IMAGE_FETCHES: usize = 6;
use std::fmt::Display;

const REPOSITORY: &str = "https://github.com/edfloreshz/toot";
const SUPPORT: &str = "https://github.com/edfloreshz/toot/issues";

/// A nav-bar destination. Some variants are not yet backed by a feature — see
/// `view`/`on_nav_select`/`subscription`'s fallback arms.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Page {
    #[default]
    Home,
    Notifications,
    Search,
    Favorites,
    Bookmarks,
    Hashtags,
    Lists,
    Explore,
    Local,
    Federated,
}

impl Display for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Page::Home => write!(f, "{}", fl!("home")),
            Page::Notifications => write!(f, "{}", fl!("notifications")),
            Page::Search => write!(f, "{}", fl!("search")),
            Page::Favorites => write!(f, "{}", fl!("favorites")),
            Page::Bookmarks => write!(f, "{}", fl!("bookmarks")),
            Page::Hashtags => write!(f, "{}", fl!("hashtags")),
            Page::Lists => write!(f, "{}", fl!("lists")),
            Page::Explore => write!(f, "{}", fl!("explore")),
            Page::Local => write!(f, "{}", fl!("local")),
            Page::Federated => write!(f, "{}", fl!("federated")),
        }
    }
}

impl Page {
    pub fn public_variants() -> Vec<Page> {
        vec![
            Self::Explore,
            Self::Local,
            Self::Federated,
            Self::Search,
            Self::Hashtags,
        ]
    }

    pub fn variants() -> Vec<Page> {
        vec![
            Self::Home,
            Self::Notifications,
            Self::Search,
            Self::Favorites,
            Self::Bookmarks,
            Self::Hashtags,
            Self::Lists,
            Self::Explore,
            Self::Local,
            Self::Federated,
        ]
    }

    pub fn icon(&self) -> widget::icon::Handle {
        if *self == Page::Hashtags {
            return widget::icon::from_svg_bytes(
                include_bytes!("../assets/icons/hashtag-symbolic.svg").as_slice(),
            )
            .symbolic(true);
        }
        widget::icon::from_name(match self {
            Page::Home => "user-home-symbolic",
            Page::Notifications => "emblem-important-symbolic",
            Page::Search => "folder-saved-search-symbolic",
            Page::Favorites => "starred-symbolic",
            Page::Bookmarks => "bookmark-new-symbolic",
            Page::Hashtags => unreachable!(),
            Page::Lists => "view-list-symbolic",
            Page::Explore => "find-location-symbolic",
            Page::Local => "network-server-symbolic",
            Page::Federated => "network-workgroup-symbolic",
        })
        .into()
    }
}

pub struct AppModel {
    generation: u64,
    reader_request: u64,
    filter_request: u64,
    filters_loading: bool,
    context_history: Vec<ContextPage>,
    thread: Option<reading::thread::Thread>,
    profile: Option<reading::profile::Profile>,
    report_serial: u64,
    core: Core,
    about: About,
    nav: nav_bar::Model,
    context_page: ContextPage,
    key_binds: HashMap<menu::KeyBind, MenuAction>,
    dialog_pages: VecDeque<Dialog>,
    dialog_editor: widget::text_editor::Content,
    config: TootConfig,
    handler: Option<cosmic_config::Config>,
    instance: String,
    onboarding: Option<onboarding::State>,
    setup_serial: u64,
    sessions_loaded: bool,
    mastodon: Client,
    /// All saved accounts (including the currently active one) and which
    /// index is active, persisted to the keychain as a whole.
    sessions: Sessions,
    cache: Cache,
    /// URLs waiting to be fetched, in priority order (newest posts' images
    /// are enqueued first). Drained at [`MAX_CONCURRENT_IMAGE_FETCHES`] at a time.
    image_queue: VecDeque<String>,
    /// URLs currently being downloaded, so we don't queue or fetch the same
    /// one twice.
    image_inflight: HashSet<String>,
    toasts: Toasts<Message>,
    /// The instance's reported maximum status length, used for the compose
    /// dialog's character counter. Defaults to Mastodon's standard limit
    /// until the real value is fetched after login.
    compose_limits: compose::Limits,
    compose_tabs: cosmic::widget::segmented_button::SingleSelectModel,
    compose_store: compose::store::Store,
    drafts: Vec<compose::State>,
    drafts_readable: bool,
    compose_request: u64,
    home: Timeline,
    notifications: notifications::Notifications,
    explore: Timeline,
    local: Timeline,
    federated: Timeline,
    favorites: Timeline,
    bookmarks: Timeline,
    hashtags: hashtags::Hashtags,
    lists: lists::Lists,
    search: search::Search,
}

#[derive(Debug, Clone)]
pub enum Message {
    Scoped(u64, Box<Message>),
    BackContext,
    RetryReader,
    ProfileMore,
    ThreadLoaded(u64, Result<(Status, megalodon::entities::Context), String>),
    ProfileLoaded(u64, Result<(Vec<Status>, Vec<Status>), String>),
    FiltersLoaded(u64, Result<Vec<reading::filters::Filter>, String>),
    MentionLoaded(u64, Result<Account, String>),
    ReloadFilters,
    PollLoaded(String, Result<megalodon::entities::Poll, String>),
    ReportComment(String),
    ReportForward(bool),
    SubmitReport,
    ReportSubmitted(u64, Result<(), String>),
    StartOnboarding,
    Onboarding(onboarding::Message),
    Open(String),
    ToggleContextPage(ContextPage),
    ToggleContextDrawer,
    UpdateConfig(TootConfig),
    InstanceEdit,
    Home(timeline::Message),
    Notifications(notifications::Message),
    Explore(timeline::Message),
    Local(timeline::Message),
    Federated(timeline::Message),
    Favorites(timeline::Message),
    Bookmarks(timeline::Message),
    Hashtags(hashtags::Message),
    Lists(lists::Message),
    Search(search::Message),
    Settings(settings::Message),
    Account(accounts::Message),
    Status(status::Message),
    Fetch(Vec<String>),
    CacheStatus(Status),
    CacheNotification(Notification),
    RemoveNotifications(Vec<String>),
    CacheRelationship(megalodon::entities::Relationship),
    CacheRelationships(Vec<megalodon::entities::Relationship>),
    CacheHandle(String, Handle),
    /// An image download failed; drop it from the in-flight set and let the
    /// next queued one start.
    ImageFetchFailed(String),
    Dialog(DialogAction),
    EditorAction(widget::text_editor::Action),
    UpdateMastodonInstance,
    /// The authenticated account, cached right after login (and at startup
    /// if a saved session is restored) so posts/relationships can be
    /// compared against "me".
    SetAccount(Account),
    /// The instance's max status length, fetched right after login.
    SetComposeLimits(compose::Limits),
    Compose(compose::Message),
    /// A status was deleted on the server; remove it from the cache.
    StatusDeleted(String),
    /// A recoverable error to surface to the user as a toast instead of
    /// only logging it and silently dropping the failed action.
    Error(String),
    CloseToast(ToastId),
    /// Periodic tick: flush every feed's cache to disk if anything changed.
    FlushCache,
    None,
}

#[derive(Debug, Clone)]
pub enum DialogAction {
    Open(Dialog),
    Update(Dialog),
    Close,
    Complete,
}

#[derive(Debug, Clone)]
pub enum Dialog {
    Drafts,
    CloseDraft,
    /// Edit the alt text of a compose attachment, by media id.
    AltText(String),
    Report(reading::report::State),
    Compose(compose::State),
    SwitchInstance(String),
    Logout,
    DeleteStatus(String),
    /// Show an image attachment full-size: (cached preview URL, original URL).
    Image(String, String),
}

pub struct Flags {
    pub config: TootConfig,
    pub handler: Option<cosmic_config::Config>,
}

impl Application for AppModel {
    type Executor = cosmic::executor::multi::Executor;
    type Flags = Flags;
    type Message = Message;
    const APP_ID: &'static str = "dev.edfloreshz.Toot";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, flags: Self::Flags) -> (Self, Task<Self::Message>) {
        let mut nav = nav_bar::Model::default();

        let instance = instance(flags.config.server.clone());

        let (sessions, sessions_loaded) = match keytar::get_password(Self::APP_ID, "mastodon-data")
        {
            Ok(data) if data.success => match Sessions::parse(&data.password) {
                Some(sessions) => (sessions, true),
                None => (Sessions::default(), false),
            },
            Ok(_) => (Sessions::default(), true),
            Err(err) => {
                tracing::error!("{err}");
                (Sessions::default(), false)
            }
        };

        let onboarding_completed = flags.config.onboarding_completed;
        let mut flags = flags;
        if let Some(preferences) = sessions
            .active_session()
            .and_then(|session| session.preferences.clone())
        {
            flags.config = preferences;
        }

        flags.config.onboarding_completed = onboarding_completed;

        let mastodon = match sessions.active_session() {
            Some(session) => Client::from_session(session),
            None => Client::new(instance.clone(), None),
        };

        let variants = (!mastodon.is_authenticated())
            .then(Page::public_variants)
            .unwrap_or_else(Page::variants);

        for page in variants {
            let id = nav
                .insert()
                .text(page.to_string())
                .icon(widget::icon(page.icon()))
                .data::<Page>(page.clone())
                .id();

            if page == Page::default() {
                nav.activate(id);
            }

            if mastodon.is_authenticated() && matches!(page, Page::Favorites | Page::Explore) {
                nav.divider_above_set(id, true);
            }
        }

        let about = About::default()
            .name(fl!("app-title"))
            .version("0.1.0")
            .icon(widget::icon::from_name(Self::APP_ID))
            .author("Eduardo Flores")
            .developers([("Eduardo Flores", "edfloreshz@proton.me")])
            .links([(fl!("repository"), REPOSITORY), (fl!("support"), SUPPORT)]);

        let mut app = AppModel {
            generation: 0,
            reader_request: 0,
            filter_request: 0,
            filters_loading: false,
            context_history: Vec::new(),
            thread: None,
            profile: None,
            report_serial: 0,
            core,
            about,
            nav,
            context_page: ContextPage::default(),
            key_binds: HashMap::new(),
            dialog_pages: VecDeque::new(),
            dialog_editor: widget::text_editor::Content::default(),
            config: flags.config.clone(),
            handler: flags.handler,
            instance: flags.config.server,
            onboarding: if sessions.sessions.is_empty() && !onboarding_completed {
                let mut state = onboarding::State::new(1, true, instance.clone(), None);
                if !sessions_loaded {
                    state.error = Some(onboarding::Failure::Keychain);
                }
                Some(state)
            } else {
                None
            },
            setup_serial: 1,
            sessions_loaded,
            mastodon: mastodon.clone(),
            sessions,
            cache: {
                let mut cache = Cache::new();
                cache.reading = reading::State::new(mastodon.is_authenticated());
                cache.hide_boosts = flags.config.hide_boosts;
                cache.hide_replies = flags.config.hide_replies;
                cache.feed_density = flags.config.feed_density;
                cache
            },
            image_queue: VecDeque::new(),
            image_inflight: HashSet::new(),
            toasts: Toasts::new(Message::CloseToast),
            compose_limits: compose::Limits::default(),
            compose_tabs: compose::panes(),
            compose_store: compose::store::Store::default(),
            drafts: vec![],
            drafts_readable: true,
            compose_request: 0,
            home: Timeline::new(mastodon.clone(), TimelineKind::Home),
            notifications: notifications::Notifications::new(mastodon.clone()),
            explore: Timeline::new(mastodon.clone(), TimelineKind::Public),
            local: Timeline::new(mastodon.clone(), TimelineKind::Local),
            federated: Timeline::new(mastodon.clone(), TimelineKind::Federated),
            favorites: Timeline::new(mastodon.clone(), TimelineKind::Favorites),
            bookmarks: Timeline::new(mastodon.clone(), TimelineKind::Bookmarks),
            hashtags: hashtags::Hashtags::new(mastodon.clone()),
            lists: lists::Lists::new(mastodon.clone()),
            search: search::Search::new(mastodon.clone()),
        };

        app.load_drafts();
        app.nav.activate_position(0);

        let mut tasks = vec![
            app.update_title(),
            app.home.load_cached(),
            app.notifications.load_cached(),
            app.explore.load_cached(),
            app.local.load_cached(),
            app.federated.load_cached(),
            app.favorites.load_cached(),
            app.bookmarks.load_cached(),
        ];
        if mastodon.is_authenticated() {
            tasks.push(app.reload_filters());
            tasks.push(fetch_session_info(mastodon));
        }

        (app, scoped_task(Task::batch(tasks), 0))
    }

    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        if self.onboarding.is_some() {
            return vec![];
        }
        let spacing = cosmic::theme::active().cosmic().spacing;
        let mut items = vec![menu::Item::Button(
            fl!("feed-settings"),
            Some(widget::icon::from_name("preferences-system-symbolic").into()),
            MenuAction::Settings,
        )];
        if self.mastodon.is_authenticated() {
            items.push(menu::Item::Button(
                format!(
                    "{} ({})",
                    fl!("compose-drafts"),
                    self.drafts
                        .iter()
                        .filter(|draft| draft.published_id.is_none())
                        .count()
                ),
                Some(widget::icon::from_name("edit-symbolic").into()),
                MenuAction::Drafts,
            ));
            items.push(menu::Item::Button(
                fl!("feed-sign-out"),
                Some(widget::icon::from_name("system-log-out-symbolic").into()),
                MenuAction::Logout,
            ));
        }
        items.push(menu::Item::Button(
            fl!("about"),
            Some(widget::icon::from_name("help-info-symbolic").into()),
            MenuAction::About,
        ));
        // Keep the root enabled; the menu bar handles opening its popup.
        let root = widget::button::icon(widget::icon::from_name("open-menu-symbolic"))
            .tooltip(format!(
                "{}\n{}",
                fl!("feed-app-menu"),
                self.sessions
                    .active_session()
                    .map(Session::label)
                    .unwrap_or_else(|| fl!("app-title"))
            ))
            .on_press(Message::None)
            .class(cosmic::style::Button::MenuRoot);
        #[cfg(target_os = "linux")]
        let root = root.name(fl!("feed-app-menu"));
        let menu_bar = menu::bar(vec![menu::Tree::with_children(
            Into::<Element<Self::Message>>::into(root),
            menu::items(&self.key_binds, items),
        )])
        .item_height(ItemHeight::Dynamic(40))
        .item_width(ItemWidth::Uniform(260))
        .spacing(spacing.space_xxxs.into());

        vec![
            menu_bar.into(),
            widget::text::heading(self.nav.text(self.nav.active()).unwrap_or("Toot"))
                .wrapping(cosmic::iced::core::text::Wrapping::None)
                .ellipsize(cosmic::iced::core::text::Ellipsize::End(
                    cosmic::iced::core::text::EllipsizeHeightLimit::Lines(1),
                ))
                .into(),
        ]
    }

    fn header_end(&self) -> Vec<Element<'_, Self::Message>> {
        if self.onboarding.is_some() {
            return vec![];
        }
        if !self.mastodon.is_authenticated() {
            vec![widget::button::standard(fl!("setup-add-account"))
                .on_press(Message::StartOnboarding)
                .into()]
        } else {
            let button = widget::button::icon(widget::icon::from_name("list-add-symbolic"))
                .icon_size(18)
                .padding(8)
                .on_press(Message::Compose(compose::Message::New));
            #[cfg(target_os = "linux")]
            let button = button.name(fl!("new-post"));
            vec![button.into()]
        }
    }

    fn nav_model(&self) -> Option<&nav_bar::Model> {
        self.onboarding.is_none().then_some(&self.nav)
    }

    fn on_nav_select(&mut self, id: nav_bar::Id) -> Task<Self::Message> {
        self.nav.activate(id);
        let mut tasks = vec![];
        match self.nav.data::<Page>(id).unwrap() {
            Page::Home => tasks.push(
                self.home
                    .update(timeline::Message::SetClient(self.mastodon.clone())),
            ),
            Page::Notifications => tasks.push(
                self.notifications
                    .update(notifications::Message::SetClient(self.mastodon.clone())),
            ),
            Page::Search => tasks.push(
                self.search
                    .update(search::Message::SetClient(self.mastodon.clone())),
            ),
            Page::Favorites => tasks.push(
                self.favorites
                    .update(timeline::Message::SetClient(self.mastodon.clone())),
            ),
            Page::Bookmarks => tasks.push(
                self.bookmarks
                    .update(timeline::Message::SetClient(self.mastodon.clone())),
            ),
            Page::Hashtags => {
                tasks.push(
                    self.hashtags
                        .update(hashtags::Message::SetClient(self.mastodon.clone())),
                );
                tasks.push(self.hashtags.update(hashtags::Message::Refresh));
            }
            Page::Lists => tasks.push(
                self.lists
                    .update(lists::Message::SetClient(self.mastodon.clone())),
            ),
            Page::Explore => tasks.push(
                self.explore
                    .update(timeline::Message::SetClient(self.mastodon.clone())),
            ),
            Page::Local => tasks.push(
                self.local
                    .update(timeline::Message::SetClient(self.mastodon.clone())),
            ),
            Page::Federated => tasks.push(
                self.federated
                    .update(timeline::Message::SetClient(self.mastodon.clone())),
            ),
        };
        tasks.push(self.update_title());
        Task::batch(tasks)
    }

    fn context_drawer(&self) -> Option<context_drawer::ContextDrawer<'_, Self::Message>> {
        if self.onboarding.is_some() || !self.core.window.show_context {
            return None;
        }

        Some(match &self.context_page {
            ContextPage::About => context_drawer::about(
                &self.about,
                |url| Message::Open(url.to_string()),
                Message::ToggleContextDrawer,
            )
            .title(self.context_page.title()),
            ContextPage::Account(account) => {
                let content = self
                    .profile
                    .as_ref()
                    .map(|profile| profile.view(&self.cache))
                    .unwrap_or_else(|| self.account(account));
                context_drawer::context_drawer(
                    self.reader_view(content),
                    Message::ToggleContextDrawer,
                )
                .title(self.context_page.title())
            }
            ContextPage::Status(status) => context_drawer::context_drawer(
                self.reader_view(self.status(status)),
                Message::ToggleContextDrawer,
            )
            .title(self.context_page.title()),
            ContextPage::Settings => {
                let content =
                    settings::view(&self.config, &self.sessions.sessions, self.sessions.active)
                        .map(Message::Settings);
                context_drawer::context_drawer(content, Message::ToggleContextDrawer)
                    .title(self.context_page.title())
            }
        })
    }

    fn dialog(&self) -> Option<Element<'_, Self::Message>> {
        if self.onboarding.is_some() {
            return None;
        }
        // The image viewer is a full-window overlay (like cosmic-files' gallery
        // view), not a small centered dialog box, so it's built and returned
        // before the `widget::dialog()`-based match below.
        if let Some(Dialog::Image(preview_url, original_url)) = self.dialog_pages.front() {
            return Some(self.view_image(preview_url.clone(), original_url.clone()));
        }

        let dialog_page = self.dialog_pages.front()?;
        if let Dialog::Report(state) = dialog_page {
            return Some(state.view());
        }

        let dialog = match dialog_page {
            Dialog::Compose(state) => {
                let preview = state
                    .in_reply_to_id
                    .as_ref()
                    .and_then(|id| self.cache.statuses.get(id))
                    .map(|post| {
                        let context = reading::filters::Context::Thread;
                        let allowed = match self.cache.reading.filters.decision(post, context) {
                            reading::filters::Decision::Allow
                            | reading::filters::Decision::Blur(_) => true,
                            reading::filters::Decision::Warn(..) => self
                                .cache
                                .reading
                                .filter_reveals
                                .contains(&(post.id.clone(), context)),
                            _ => false,
                        };
                        if !allowed {
                            return widget::text(fl!("reading-filtered")).into();
                        }
                        let text = if self.cache.reading.content_visible(post) {
                            reading::html::plain(&post.content)
                        } else {
                            post.spoiler_text.clone()
                        };
                        widget::column![
                            widget::text::caption(reading::html::full_acct(&post.account)),
                            widget::selectable_text(text)
                        ]
                        .spacing(4)
                        .into()
                    });
                compose::view(
                    state,
                    &self.dialog_editor,
                    &self.compose_tabs,
                    preview,
                    &self.compose_limits,
                    self.cache
                        .me
                        .as_ref()
                        .map(reading::html::full_acct)
                        .unwrap_or_default(),
                    &self.mastodon,
                    &self.compose_store,
                )
            }
            Dialog::Drafts => {
                let drafts: Vec<_> = self
                    .drafts
                    .iter()
                    .filter(|draft| draft.published_id.is_none())
                    .rev()
                    .collect();
                let mut content = widget::column![].spacing(12);
                if !self.drafts_readable {
                    content = content.push(widget::text(fl!("compose-storage-error")));
                }
                for draft in &drafts {
                    content = content.push(compose::draft_card(draft));
                }
                if drafts.is_empty() {
                    content = content.push(
                        widget::container(
                            widget::column![
                                widget::icon::from_name("edit-symbolic").size(48),
                                widget::text::heading(fl!("compose-no-drafts")),
                                widget::text::caption(fl!("compose-no-drafts-hint")),
                            ]
                            .spacing(8)
                            .align_x(cosmic::iced::Alignment::Center),
                        )
                        .center_x(Length::Fill)
                        .padding(32),
                    );
                }
                widget::dialog()
                    .title(fl!("compose-drafts"))
                    .control(
                        widget::container(widget::scrollable(content))
                            .height(Length::Fill)
                            .width(Length::Fill),
                    )
                    .height(Length::Fill)
                    .max_height(520)
                    .primary_action(
                        widget::button::standard(fl!("compose-close"))
                            .on_press(Message::Dialog(DialogAction::Close)),
                    )
            }
            Dialog::CloseDraft => widget::dialog()
                .title(fl!("compose-close-question"))
                .body(fl!("compose-close-description"))
                .primary_action(
                    widget::button::suggested(fl!("compose-keep"))
                        .on_press(Message::Compose(compose::Message::Keep)),
                )
                .secondary_action(
                    widget::button::standard(fl!("compose-continue"))
                        .on_press(Message::Compose(compose::Message::ContinueEditing)),
                )
                .tertiary_action(
                    widget::button::destructive(fl!("compose-discard"))
                        .on_press(Message::Compose(compose::Message::Discard)),
                ),
            Dialog::SwitchInstance(instance) => self.switch_instance(instance.clone()),
            Dialog::AltText(id) => match self.active_compose() {
                Some(state) => compose::alt_view(
                    state,
                    id,
                    &self.compose_limits,
                    &self.mastodon,
                    &self.compose_store,
                ),
                None => return None,
            },
            Dialog::Logout => self.logout(),
            Dialog::DeleteStatus(id) => self.delete_status(id.clone()),
            Dialog::Image(..) | Dialog::Report(_) => unreachable!("handled above"),
        };

        Some(dialog.into())
    }

    fn on_escape(&mut self) -> Task<Self::Message> {
        if self.onboarding.is_some() {
            return Task::none();
        }
        if matches!(self.dialog_pages.front(), Some(Dialog::Compose(_))) {
            return self.update(Message::Compose(compose::Message::Close));
        }
        if matches!(self.dialog_pages.front(), Some(Dialog::Report(state)) if state.sending) {
            return Task::none();
        }
        if self.dialog_pages.pop_front().is_some() {
            return Task::none();
        }

        if self.core.window.show_context {
            self.core.window.show_context = false;
        }

        Task::none()
    }

    /// Flush the cache before the app closes — otherwise it's only ever
    /// saved by the periodic 30s tick, so a quick open-then-quit would never
    /// persist anything.
    fn on_app_exit(&mut self) -> Option<Self::Message> {
        if !self.persist_active_draft() {
            return Some(Message::None);
        }
        self.flush_cache_to_disk();
        if let Some(session) = self.sessions.sessions.get_mut(self.sessions.active) {
            session.preferences = Some(self.config.clone());
            let _ = self.persist_sessions();
        }
        None
    }

    fn view(&self) -> Element<'_, Self::Message> {
        if let Some(state) = &self.onboarding {
            return widget::toaster(
                &self.toasts,
                widget::container(
                    state
                        .view(&self.sessions.sessions, &self.cache)
                        .map(Message::Onboarding),
                )
                .center_x(Length::Fill)
                .height(Length::Fill),
            );
        }
        let content = match self.nav.active_data::<Page>() {
            Some(page) => match page {
                Page::Home => self.home.view(&self.cache).map(Message::Home),
                Page::Notifications => self
                    .notifications
                    .view(&self.cache)
                    .map(Message::Notifications),
                Page::Explore => self.explore.view(&self.cache).map(Message::Explore),
                Page::Local => self.local.view(&self.cache).map(Message::Local),
                Page::Federated => self.federated.view(&self.cache).map(Message::Federated),
                Page::Favorites => self.favorites.view(&self.cache).map(Message::Favorites),
                Page::Bookmarks => self.bookmarks.view(&self.cache).map(Message::Bookmarks),
                Page::Hashtags => self.hashtags.view(&self.cache).map(Message::Hashtags),
                Page::Lists => self.lists.view(&self.cache).map(Message::Lists),
                Page::Search => self.search.view(&self.cache).map(Message::Search),
            },
            None => widget::text("Select a page").into(),
        }
        .apply(widget::container)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center);

        widget::toaster(&self.toasts, content)
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let mut subscriptions = vec![self
            .core()
            .watch_config::<TootConfig>(Self::APP_ID)
            .map(|update| Message::UpdateConfig(update.config))];

        if self.onboarding.is_some() {
            return Subscription::batch(subscriptions);
        }

        match self.nav.active_data::<Page>() {
            Some(Page::Home) => subscriptions.push(self.home.subscription().map(Message::Home)),
            Some(Page::Notifications) => subscriptions.push(
                self.notifications
                    .subscription()
                    .map(Message::Notifications),
            ),
            Some(Page::Search) => {
                subscriptions.push(self.search.subscription().map(Message::Search))
            }
            Some(Page::Favorites) => {
                subscriptions.push(self.favorites.subscription().map(Message::Favorites))
            }
            Some(Page::Bookmarks) => {
                subscriptions.push(self.bookmarks.subscription().map(Message::Bookmarks))
            }
            Some(Page::Hashtags) => {
                subscriptions.push(self.hashtags.subscription().map(Message::Hashtags))
            }
            Some(Page::Lists) => subscriptions.push(self.lists.subscription().map(Message::Lists)),
            Some(Page::Explore) => {
                subscriptions.push(self.explore.subscription().map(Message::Explore))
            }
            Some(Page::Local) => subscriptions.push(self.local.subscription().map(Message::Local)),
            Some(Page::Federated) => {
                subscriptions.push(self.federated.subscription().map(Message::Federated))
            }
            None => (),
        };

        // Preload these pages in the background so they're ready on first visit.
        let active = self.nav.active_data::<Page>();
        if active != Some(&Page::Notifications) {
            subscriptions.push(
                self.notifications
                    .subscription()
                    .map(Message::Notifications),
            );
        }
        if active != Some(&Page::Search) {
            subscriptions.push(self.search.subscription().map(Message::Search));
        }
        if active != Some(&Page::Favorites) {
            subscriptions.push(self.favorites.subscription().map(Message::Favorites));
        }
        if active != Some(&Page::Bookmarks) {
            subscriptions.push(self.bookmarks.subscription().map(Message::Bookmarks));
        }
        if active != Some(&Page::Hashtags) {
            subscriptions.push(self.hashtags.subscription().map(Message::Hashtags));
        }
        if active != Some(&Page::Lists) {
            subscriptions.push(self.lists.subscription().map(Message::Lists));
        }
        if !self.mastodon.is_authenticated() {
            if active != Some(&Page::Local) {
                subscriptions.push(self.local.subscription().map(Message::Local));
            }
            if active != Some(&Page::Federated) {
                subscriptions.push(self.federated.subscription().map(Message::Federated));
            }
        }

        if self.mastodon.is_authenticated() {
            subscriptions.push(
                cosmic::iced::time::every(std::time::Duration::from_secs(60))
                    .map(|_| Message::ReloadFilters),
            );
            subscriptions.push(crate::streaming::stream_user_events(self.mastodon.clone()));
        }

        subscriptions.push(
            cosmic::iced::time::every(std::time::Duration::from_secs(30))
                .map(|_| Message::FlushCache),
        );

        if self
            .active_compose()
            .is_some_and(|state| state.media.iter().any(|media| media.uploading))
        {
            subscriptions.push(
                cosmic::iced::time::every(std::time::Duration::from_millis(200))
                    .map(|_| Message::Compose(compose::Message::Tick)),
            );
        }
        let generation = self.generation;
        Subscription::batch(subscriptions)
            .with(generation)
            .map(|(generation, message)| Message::Scoped(generation, Box::new(message)))
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        let mut tasks = vec![];
        match message {
            Message::Scoped(generation, message) => {
                if generation != self.generation {
                    return Task::none();
                }
                return self.update(*message);
            }
            Message::BackContext => {
                if let Some(page) = self.context_history.pop() {
                    self.context_page = page;
                    self.core.window.show_context = true;
                    tasks.push(self.load_reader());
                }
            }
            Message::RetryReader => tasks.push(self.load_reader()),
            Message::ProfileMore => {
                if let Some(profile) = &mut self.profile {
                    if !profile.loading {
                        profile.loading = true;
                        profile.error = None;
                        let (request, id, max_id) = (
                            profile.request,
                            profile.account.id.clone(),
                            profile.max_id.clone(),
                        );
                        let client = self.mastodon.clone();
                        tasks.push(cosmic::task::future(async move {
                            Message::ProfileLoaded(
                                request,
                                reading::profile::fetch(client, id, max_id).await,
                            )
                        }));
                    }
                }
            }
            Message::ThreadLoaded(request, result) => {
                if let Some(thread) = &mut self.thread {
                    if thread.request == request {
                        thread.loading = false;
                        match result {
                            Ok((post, context)) => {
                                thread.ancestors = context
                                    .ancestors
                                    .iter()
                                    .map(|post| post.id.clone())
                                    .collect();
                                thread.descendants = context
                                    .descendants
                                    .iter()
                                    .map(|post| post.id.clone())
                                    .collect();
                                let mut images = vec![];
                                for post in context
                                    .ancestors
                                    .into_iter()
                                    .chain(std::iter::once(post))
                                    .chain(context.descendants)
                                {
                                    images.extend(crate::cache::extract_status_images(&post));
                                    self.cache.insert_status(post);
                                }
                                tasks.push(cosmic::task::message(Message::Fetch(images)));
                            }
                            Err(error) => thread.error = Some(error),
                        }
                    }
                }
            }
            Message::ProfileLoaded(request, result) => {
                if let Some(profile) = &mut self.profile {
                    if profile.request == request {
                        profile.loading = false;
                        match result {
                            Ok((pinned, posts)) => {
                                profile.append(&pinned, &posts);
                                let mut images = vec![];
                                for post in pinned.into_iter().chain(posts) {
                                    images.extend(crate::cache::extract_status_images(&post));
                                    self.cache.insert_status(post);
                                }
                                tasks.push(cosmic::task::message(Message::Fetch(images)));
                            }
                            Err(error) => profile.error = Some(error),
                        }
                    }
                }
            }
            Message::ReloadFilters => {
                if self.mastodon.is_authenticated() {
                    tasks.push(self.reload_filters());
                }
            }
            Message::FiltersLoaded(request, result) => {
                if request == self.filter_request {
                    self.filters_loading = false;
                    match result {
                        Ok(filters) => {
                            if !self.cache.reading.filters.same_definitions(&filters) {
                                self.cache.reading.filter_reveals.clear();
                                self.cache.reading.media.clear();
                            }
                            self.cache.reading.filters = reading::filters::Filters::ready(filters);
                        }
                        Err(_) => self.cache.reading.filters = reading::filters::Filters::Failed,
                    }
                }
            }
            Message::MentionLoaded(request, result) => {
                if request == self.reader_request {
                    match result {
                        Ok(account) => tasks.push(
                            self.update(Message::ToggleContextPage(ContextPage::Account(account))),
                        ),
                        Err(error) => tasks.push(self.update(Message::Error(error))),
                    }
                }
            }
            Message::PollLoaded(id, result) => {
                self.cache.reading.poll_busy.remove(&id);
                match result {
                    Ok(poll) => {
                        self.cache.reading.polls.insert(id.clone(), poll);
                        self.cache.reading.choices.remove(&id);
                        self.cache.reading.poll_errors.remove(&id);
                    }
                    Err(error) => {
                        self.cache.reading.poll_errors.insert(id, error);
                    }
                }
            }
            Message::ReportComment(comment) => {
                if let Some(Dialog::Report(state)) = self.dialog_pages.front_mut() {
                    if !state.sending {
                        state.comment = comment;
                    }
                }
            }
            Message::ReportForward(forward) => {
                if let Some(Dialog::Report(state)) = self.dialog_pages.front_mut() {
                    if !state.sending {
                        state.forward = forward;
                    }
                }
            }
            Message::SubmitReport => {
                if let Some(Dialog::Report(state)) = self.dialog_pages.front_mut() {
                    if state.can_submit() && self.mastodon.is_authenticated() {
                        state.sending = true;
                        state.error = None;
                        let state = state.clone();
                        let client = self.mastodon.clone();
                        tasks.push(cosmic::task::future(async move {
                            let options = state.options();
                            let result = tokio::time::timeout(
                                std::time::Duration::from_secs(30),
                                client.report(state.account.id, Some(&options)),
                            )
                            .await
                            .ok()
                            .and_then(Result::ok)
                            .map(|_| ())
                            .ok_or_else(|| fl!("reading-report-error"));
                            Message::ReportSubmitted(state.request, result)
                        }));
                    }
                }
            }
            Message::ReportSubmitted(request, result) => {
                if let Some(Dialog::Report(state)) = self.dialog_pages.front_mut() {
                    if state.request == request {
                        match result {
                            Ok(()) => {
                                self.dialog_pages.pop_front();
                                tasks.push(
                                    self.toasts
                                        .push(Toast::new(fl!("reading-report-sent")))
                                        .map(cosmic::Action::App),
                                );
                            }
                            Err(error) => {
                                state.sending = false;
                                state.error = Some(error);
                            }
                        }
                    }
                }
            }
            Message::Home(message) => {
                self.cache.dirty = true;
                tasks.push(self.home.update(message));
            }
            Message::Notifications(message) => {
                self.cache.dirty = true;
                tasks.push(self.notifications.update(message));
            }
            Message::Explore(message) => {
                self.cache.dirty = true;
                tasks.push(self.explore.update(message.clone()));
            }
            Message::Local(message) => {
                self.cache.dirty = true;
                tasks.push(self.local.update(message));
            }
            Message::Federated(message) => {
                self.cache.dirty = true;
                tasks.push(self.federated.update(message));
            }
            Message::Favorites(message) => {
                self.cache.dirty = true;
                tasks.push(self.favorites.update(message));
            }
            Message::Bookmarks(message) => {
                self.cache.dirty = true;
                tasks.push(self.bookmarks.update(message));
            }
            Message::Hashtags(message) => {
                tasks.push(self.hashtags.update(message));
            }
            Message::Lists(message) => {
                tasks.push(self.lists.update(message));
            }
            Message::Search(message) => {
                tasks.push(self.search.update(message));
            }
            Message::Settings(message) => match message {
                settings::Message::ToggleHideBoosts(hide) => {
                    self.config.hide_boosts = hide;
                    self.cache.hide_boosts = hide;
                    if let Some(ref handler) = self.handler {
                        if let Err(err) = self.config.set_hide_boosts(handler, hide) {
                            tracing::error!("{err}");
                        }
                    }
                }
                settings::Message::ToggleHideReplies(hide) => {
                    self.config.hide_replies = hide;
                    self.cache.hide_replies = hide;
                    if let Some(ref handler) = self.handler {
                        if let Err(err) = self.config.set_hide_replies(handler, hide) {
                            tracing::error!("{err}");
                        }
                    }
                }
                settings::Message::SetFeedDensity(density) => {
                    self.config.feed_density = density;
                    self.cache.feed_density = density;
                    if let Some(ref handler) = self.handler {
                        if let Err(err) = self.config.set_feed_density(handler, density) {
                            tracing::error!("{err}");
                        }
                    }
                }
                settings::Message::SetThemeMode(mode) => {
                    self.config.theme_mode = mode;
                    if let Some(ref handler) = self.handler {
                        if let Err(err) = self.config.set_theme_mode(handler, mode) {
                            tracing::error!("{err}");
                        }
                    }
                    return cosmic::command::set_theme(self.config.theme_mode.theme());
                }
                settings::Message::SwitchAccount(index) => tasks.push(self.switch_account(index)),
                settings::Message::RemoveAccount(index) => tasks.push(self.remove_account(index)),
                settings::Message::AddAccount => tasks.push(self.update(Message::StartOnboarding)),
            },
            Message::Account(message) => match message {
                accounts::Message::Follow(id, following) => {
                    let mastodon = self.mastodon.clone();
                    tasks.push(cosmic::task::future(async move {
                        let result = if following {
                            mastodon.unfollow_account(id).await
                        } else {
                            mastodon.follow_account(id, None).await
                        };
                        match result {
                            Ok(response) => Message::CacheRelationship(response.json),
                            Err(err) => Message::Error(format!("Couldn't update follow: {err}")),
                        }
                    }))
                }
                accounts::Message::Mute(id, muting) => {
                    let mastodon = self.mastodon.clone();
                    tasks.push(cosmic::task::future(async move {
                        let result = if muting {
                            mastodon.unmute_account(id).await
                        } else {
                            mastodon.mute_account(id, true).await
                        };
                        match result {
                            Ok(response) => Message::CacheRelationship(response.json),
                            Err(err) => Message::Error(format!("Couldn't update mute: {err}")),
                        }
                    }))
                }
                accounts::Message::Block(id, blocking) => {
                    let mastodon = self.mastodon.clone();
                    tasks.push(cosmic::task::future(async move {
                        let result = if blocking {
                            mastodon.unblock_account(id).await
                        } else {
                            mastodon.block_account(id).await
                        };
                        match result {
                            Ok(response) => Message::CacheRelationship(response.json),
                            Err(err) => Message::Error(format!("Couldn't update block: {err}")),
                        }
                    }))
                }
                _ => tasks.push(accounts::update(message)),
            },
            Message::Status(message) => match message {
                status::Message::ToggleWarning(id) => {
                    if !self.cache.reading.warnings.remove(&id) {
                        self.cache.reading.warnings.insert(id);
                    }
                }
                status::Message::ToggleMedia(id) => {
                    if !self.cache.reading.media.remove(&id) {
                        self.cache.reading.media.insert(id);
                    }
                }
                status::Message::ToggleFilter(id, context) => {
                    if !self
                        .cache
                        .reading
                        .filter_reveals
                        .remove(&(id.clone(), context))
                    {
                        self.cache.reading.filter_reveals.insert((id, context));
                    }
                }
                status::Message::ReloadFilters => tasks.push(self.update(Message::ReloadFilters)),
                status::Message::Copy(text) => tasks.push(cosmic::iced::clipboard::write(text)),
                status::Message::InlineLink(id, url) => {
                    let mention = self
                        .cache
                        .statuses
                        .get(&id)
                        .and_then(|post| post.mentions.iter().find(|mention| mention.url == url))
                        .map(|mention| mention.id.clone());
                    if let Some(id) = mention {
                        self.reader_request += 1;
                        let request = self.reader_request;
                        let client = self.mastodon.clone();
                        tasks.push(cosmic::task::future(async move {
                            let result = tokio::time::timeout(
                                std::time::Duration::from_secs(30),
                                client.get_account(id),
                            )
                            .await
                            .ok()
                            .and_then(Result::ok)
                            .map(|response| response.json)
                            .ok_or_else(|| fl!("reading-profile-error"));
                            Message::MentionLoaded(request, result)
                        }));
                    } else {
                        tasks.push(self.update(Message::Open(url)));
                    }
                }
                status::Message::Report(account, status_id) => {
                    if self.mastodon.is_authenticated() {
                        self.report_serial += 1;
                        self.dialog_pages
                            .push_front(Dialog::Report(reading::report::State::new(
                                self.report_serial,
                                account,
                                status_id,
                            )));
                    }
                }
                status::Message::PollChoice(id, choice) => {
                    if let Some(poll) = self.find_poll(&id).cloned() {
                        if reading::poll_open(&poll)
                            && !poll.voted.unwrap_or(false)
                            && !self.cache.reading.poll_busy.contains(&id)
                        {
                            let choices = self.cache.reading.choices.entry(id).or_default();
                            if poll.multiple {
                                if choices.contains(&choice) {
                                    choices.retain(|value| *value != choice);
                                } else if (choice as usize) < poll.options.len() {
                                    choices.push(choice);
                                }
                            } else if (choice as usize) < poll.options.len() {
                                *choices = vec![choice];
                            }
                        }
                    }
                }
                status::Message::Vote(status_id, id) => {
                    let choices = self
                        .cache
                        .reading
                        .choices
                        .get(&id)
                        .cloned()
                        .unwrap_or_default();
                    let allowed = self.mastodon.is_authenticated()
                        && !self.cache.reading.poll_busy.contains(&id)
                        && self.find_poll(&id).is_some_and(|poll| {
                            reading::poll_open(poll)
                                && !poll.voted.unwrap_or(false)
                                && reading::valid_choices(poll, &choices)
                        });
                    if allowed {
                        self.cache.reading.poll_busy.insert(id.clone());
                        let client = self.mastodon.clone();
                        tasks.push(cosmic::task::future(async move {
                            let result = tokio::time::timeout(
                                std::time::Duration::from_secs(30),
                                client.vote_poll(id.clone(), choices, Some(status_id)),
                            )
                            .await
                            .ok()
                            .and_then(Result::ok)
                            .map(|response| response.json)
                            .ok_or_else(|| fl!("reading-poll-error"));
                            Message::PollLoaded(id, result)
                        }));
                    }
                }
                status::Message::RefreshPoll(id) => {
                    if self.cache.reading.poll_busy.insert(id.clone()) {
                        let client = self.mastodon.clone();
                        tasks.push(cosmic::task::future(async move {
                            let result = tokio::time::timeout(
                                std::time::Duration::from_secs(30),
                                client.get_poll(id.clone()),
                            )
                            .await
                            .ok()
                            .and_then(Result::ok)
                            .map(|response| response.json)
                            .ok_or_else(|| fl!("reading-poll-error"));
                            Message::PollLoaded(id, result)
                        }));
                    }
                }

                status::Message::Favorite(status_id, favorited) => {
                    let mastodon = self.mastodon.clone();
                    tasks.push(cosmic::task::future(async move {
                        let result = if favorited {
                            mastodon.unfavourite_status(status_id).await
                        } else {
                            mastodon.favourite_status(status_id).await
                        };
                        match result {
                            Ok(response) => Message::CacheStatus(response.json),
                            Err(err) => Message::Error(format!("Couldn't update favorite: {err}")),
                        }
                    }))
                }
                status::Message::Boost(status_id, boosted) => {
                    let mastodon = self.mastodon.clone();
                    tasks.push(cosmic::task::future(async move {
                        let result = if boosted {
                            mastodon.unreblog_status(status_id).await
                        } else {
                            mastodon.reblog_status(status_id).await
                        };
                        match result {
                            Ok(response) => Message::CacheStatus(response.json),
                            Err(err) => Message::Error(format!("Couldn't update boost: {err}")),
                        }
                    }))
                }
                status::Message::Bookmark(status_id, bookmarked) => {
                    let mastodon = self.mastodon.clone();
                    tasks.push(cosmic::task::future(async move {
                        let result = if bookmarked {
                            mastodon.unbookmark_status(status_id).await
                        } else {
                            mastodon.bookmark_status(status_id).await
                        };
                        match result {
                            Ok(response) => Message::CacheStatus(response.json),
                            Err(err) => Message::Error(format!("Couldn't update bookmark: {err}")),
                        }
                    }))
                }
                _ => tasks.push(status::update(message)),
            },
            Message::CacheHandle(url, handle) => {
                self.image_inflight.remove(&url);
                self.cache.insert_handle(url, handle);
                tasks.push(self.drain_image_queue());
            }
            Message::ImageFetchFailed(url) => {
                self.image_inflight.remove(&url);
                tasks.push(self.drain_image_queue());
            }
            Message::CacheStatus(status) => {
                self.cache.insert_status(status.clone());
            }
            Message::RemoveNotifications(ids) => {
                self.cache.notifications.retain(|id, _| !ids.contains(id));
                self.cache.dirty = true;
            }
            Message::CacheNotification(notification) => {
                self.cache.insert_notification(notification.clone());
            }
            Message::CacheRelationship(relationship) => {
                self.cache.insert_relationship(relationship);
            }
            Message::CacheRelationships(relationships) => {
                for relationship in relationships {
                    self.cache.insert_relationship(relationship);
                }
            }
            Message::Fetch(urls) => {
                for url in urls {
                    if !self.cache.handles.contains_key(&url)
                        && !self.image_inflight.contains(&url)
                        && !self.image_queue.contains(&url)
                    {
                        self.image_queue.push_back(url);
                    }
                }
                tasks.push(self.drain_image_queue());
            }
            Message::InstanceEdit => {
                let instance = self.instance.clone();
                if let Some(ref handler) = self.handler {
                    if let Err(err) = self.config.set_server(handler, instance) {
                        tracing::error!("{err}")
                    }
                }
            }
            Message::StartOnboarding => {
                if self.onboarding.is_none() {
                    self.setup_serial += 1;
                    self.core.window.show_context = false;
                    self.onboarding = Some(onboarding::State::new(
                        self.setup_serial,
                        false,
                        self.mastodon.base_url.clone(),
                        self.sessions.active_session().map(|_| self.sessions.active),
                    ));
                }
            }
            Message::Onboarding(message) => {
                if let Some(state) = &mut self.onboarding {
                    let effect = state.update(message);
                    tasks.push(self.apply_onboarding_effect(effect));
                }
            }
            Message::UpdateMastodonInstance => {
                self.mastodon = Client::new(self.instance(), None);
            }
            Message::SetAccount(account) => {
                // Migrate legacy sessions after credentials have been verified.
                if let Some(session) = self.sessions.sessions.get_mut(self.sessions.active) {
                    if session.account_id.is_empty() {
                        session.account_id = account.id.clone();
                        session.acct = account.acct.clone();
                        self.mastodon.account_id = account.id.clone();
                        let _ = self.persist_sessions();
                        tasks.push(self.reset_and_reload_feeds());
                    }
                }
                self.cache.me = Some(account);
            }
            Message::SetComposeLimits(limits) => {
                self.compose_limits = limits;
            }
            Message::Compose(message) => tasks.push(self.update_compose(message)),
            Message::StatusDeleted(id) => {
                self.cache.statuses.remove(&id);
                self.cache.dirty = true;
                for feed in [
                    &mut self.home,
                    &mut self.explore,
                    &mut self.local,
                    &mut self.federated,
                    &mut self.favorites,
                    &mut self.bookmarks,
                ] {
                    tasks.push(feed.update(timeline::Message::DeleteStatus(id.clone())));
                }
            }
            Message::Open(url) => {
                if !reading::html::web_url(&url) {
                    return Task::none();
                }
                if let Err(err) = open::that_detached(url) {
                    tracing::error!("{err}")
                }
            }
            Message::ToggleContextPage(context_page) => {
                if self.context_page == context_page {
                    self.core.window.show_context = !self.core.window.show_context;
                } else {
                    if let ContextPage::Account(account) = &context_page {
                        if self.mastodon.is_authenticated() {
                            let mastodon = self.mastodon.clone();
                            let id = account.id.clone();
                            tasks.push(cosmic::task::future(async move {
                                match mastodon.get_relationships(vec![id]).await {
                                    Ok(response) => response
                                        .json
                                        .into_iter()
                                        .next()
                                        .map(Message::CacheRelationship)
                                        .unwrap_or(Message::None),
                                    Err(err) => {
                                        Message::Error(format!("Couldn't load relationship: {err}"))
                                    }
                                }
                            }));
                        }
                    }
                    if !self.core.window.show_context {
                        self.context_history.clear();
                    }
                    if self.core.window.show_context
                        && matches!(
                            self.context_page,
                            ContextPage::Account(_) | ContextPage::Status(_)
                        )
                    {
                        self.context_history.push(self.context_page.clone());
                    }
                    self.context_page = context_page;
                    self.core.window.show_context = true;
                    tasks.push(self.load_reader());
                }
            }
            Message::ToggleContextDrawer => {
                self.core.window.show_context = !self.core.window.show_context;
            }
            Message::Dialog(action) => match action {
                DialogAction::Open(dialog) => match dialog {
                    Dialog::Compose(state) => return self.open_compose(state),
                    _ => self.dialog_pages.push_back(dialog),
                },
                DialogAction::Update(dialog_page) => {
                    self.dialog_pages[0] = dialog_page;
                }
                DialogAction::Close => {
                    if matches!(self.dialog_pages.front(), Some(Dialog::Compose(_))) {
                        return self.update(Message::Compose(compose::Message::Close));
                    }
                    if !matches!(self.dialog_pages.front(), Some(Dialog::Report(state)) if state.sending)
                    {
                        self.dialog_pages.pop_front();
                    }
                }
                DialogAction::Complete => {
                    if let Some(dialog_page) = self.dialog_pages.pop_front() {
                        match dialog_page {
                            Dialog::Compose(state) => {
                                self.dialog_pages.push_front(Dialog::Compose(state));
                                tasks.push(self.update(Message::Compose(compose::Message::Submit)));
                            }
                            Dialog::Drafts | Dialog::CloseDraft | Dialog::AltText(_) => {}
                            Dialog::DeleteStatus(id) => {
                                let mastodon = self.mastodon.clone();
                                tasks.push(cosmic::task::future(async move {
                                    match mastodon.delete_status(id.clone()).await {
                                        Ok(_) => Message::StatusDeleted(id),
                                        Err(err) => {
                                            Message::Error(format!("Couldn't delete post: {err}"))
                                        }
                                    }
                                }));
                            }
                            Dialog::SwitchInstance(instance) => {
                                self.instance = instance;
                                tasks.push(self.update(Message::InstanceEdit));
                                tasks.push(self.update(Message::UpdateMastodonInstance))
                            }
                            Dialog::Logout => {
                                tasks.push(self.remove_account(self.sessions.active));
                            }
                            Dialog::Image(..) => {}
                            Dialog::Report(state) => {
                                self.dialog_pages.push_front(Dialog::Report(state));
                            }
                        }
                    }
                }
            },
            Message::EditorAction(action) => {
                if self.active_compose().is_some_and(compose::State::editable) {
                    let editing = action.is_edit();
                    self.dialog_editor.perform(action);
                    if editing {
                        let text = self.dialog_editor.text();
                        if let Some(state) = self.active_compose_mut() {
                            state.text = Some(text);
                            state.changed();
                        }
                        self.persist_active_draft();
                    }
                }
            }
            Message::UpdateConfig(config) => {
                self.config = config;
            }
            Message::Error(message) => {
                tracing::error!("{message}");
                tasks.push(
                    self.toasts
                        .push(Toast::new(message))
                        .map(cosmic::Action::App),
                );
            }
            Message::CloseToast(id) => {
                self.toasts.remove(id);
            }
            Message::FlushCache => self.flush_cache_to_disk(),
            Message::None => (),
        }
        scoped_task(Task::batch(tasks), self.generation)
    }
}

impl AppModel {
    fn active_compose(&self) -> Option<&compose::State> {
        self.dialog_pages.iter().find_map(|dialog| {
            if let Dialog::Compose(state) = dialog {
                Some(state)
            } else {
                None
            }
        })
    }
    fn active_compose_mut(&mut self) -> Option<&mut compose::State> {
        self.dialog_pages.iter_mut().find_map(|dialog| {
            if let Dialog::Compose(state) = dialog {
                Some(state)
            } else {
                None
            }
        })
    }
    fn load_drafts(&mut self) {
        if self.mastodon.account_id.is_empty() {
            self.drafts.clear();
            self.drafts_readable = true;
            return;
        }
        match self.compose_store.load(&self.mastodon.cache_key()) {
            Ok(drafts) => {
                self.drafts = drafts;
                self.drafts_readable = true;
            }
            Err(_) => {
                self.drafts.clear();
                self.drafts_readable = false;
            }
        }
    }
    fn persist_active_draft(&mut self) -> bool {
        let Some(state) = self.active_compose().cloned() else {
            return true;
        };
        let key = self.mastodon.cache_key();
        if !self.drafts_readable {
            match self.compose_store.load(&key) {
                Ok(mut saved) => {
                    for draft in self.drafts.drain(..) {
                        saved.retain(|old| old.id != draft.id);
                        saved.push(draft);
                    }
                    self.drafts = saved;
                    self.drafts_readable = true;
                }
                Err(error) => {
                    if let Some(state) = self.active_compose_mut() {
                        state.storage_error = Some(error);
                    }
                    return false;
                }
            }
        }
        if let Some(index) = self.drafts.iter().position(|draft| draft.id == state.id) {
            self.drafts[index] = state;
        } else {
            self.drafts.push(state);
        }
        let result = self.compose_store.save(&key, &self.drafts);
        if let Some(state) = self.active_compose_mut() {
            state.storage_error = result.as_ref().err().cloned();
        }
        result.is_ok()
    }
    fn open_compose(&mut self, state: compose::State) -> Task<Message> {
        if self.active_compose().is_some() {
            return cosmic::task::message(Message::Error(fl!("compose-existing")));
        }
        if self.cache.me.is_none() || self.mastodon.account_id.is_empty() {
            return cosmic::task::message(Message::Error(fl!("compose-account-loading")));
        }
        self.compose_request += 1;
        self.dialog_editor =
            widget::text_editor::Content::with_text(state.text.as_deref().unwrap_or(""));
        self.dialog_pages
            .retain(|dialog| !matches!(dialog, Dialog::Drafts));
        self.dialog_pages.push_front(Dialog::Compose(state));
        self.persist_active_draft();
        Task::none()
    }
    fn close_compose_frames(&mut self) {
        self.compose_request += 1;
        self.dialog_pages.retain(|dialog| {
            !matches!(
                dialog,
                Dialog::Compose(_) | Dialog::CloseDraft | Dialog::AltText(_)
            )
        });
        self.dialog_editor = widget::text_editor::Content::default();
    }
    fn drop_active_draft(&mut self) -> bool {
        let Some(id) = self.active_compose().map(|state| state.id.clone()) else {
            return true;
        };
        match self.drop_draft(&id) {
            Ok(()) => true,
            Err(error) => {
                if let Some(state) = self.active_compose_mut() {
                    state.storage_error = Some(error);
                }
                false
            }
        }
    }
    fn drop_draft(&mut self, id: &str) -> Result<(), String> {
        let candidate: Vec<_> = self
            .drafts
            .iter()
            .filter(|draft| draft.id != id)
            .cloned()
            .collect();
        let key = self.mastodon.cache_key();
        self.compose_store.save(&key, &candidate)?;
        self.drafts = candidate;
        self.compose_store.remove_assets(&key, id);
        Ok(())
    }
    fn begin_status_submission(&mut self) -> Task<Message> {
        let previous_attempt = self.active_compose().and_then(|state| state.attempted_at);
        if let Some(state) = self.active_compose_mut() {
            state
                .attempted_at
                .get_or_insert(chrono::Utc::now().timestamp());
            state.error = None;
        }
        if !self.persist_active_draft() {
            if let Some(state) = self.active_compose_mut() {
                state.attempted_at = previous_attempt;
            }
            return Task::none();
        }
        let Some(state) = self.active_compose().cloned() else {
            return Task::none();
        };
        if let Some(current) = self.active_compose_mut() {
            current.sending = true;
        }
        let client = self.mastodon.clone();
        let request = self.compose_request;
        cosmic::task::future(async move {
            Message::Compose(compose::Message::Submitted(
                request,
                compose::network::submit(client, state).await,
            ))
        })
    }
    fn update_compose(&mut self, message: compose::Message) -> Task<Message> {
        use compose::Message as C;
        match message {
            C::New => return self.open_compose(compose::State::new(self.cache.me.as_ref())),
            C::Reply(id) => {
                if let Some(post) = self.cache.statuses.get(&id) {
                    return self
                        .open_compose(compose::State::reply_to(post, self.cache.me.as_ref()));
                }
                return cosmic::task::message(Message::Error(fl!("compose-reply-error")));
            }
            C::Edit(id) => {
                if self.active_compose().is_some() {
                    return cosmic::task::message(Message::Error(fl!("compose-existing")));
                }
                if !self.cache.is_me(
                    self.cache
                        .statuses
                        .get(&id)
                        .map(|post| post.account.id.as_str())
                        .unwrap_or(""),
                ) {
                    return Task::none();
                }
                self.compose_request += 1;
                let request = self.compose_request;
                let client = self.mastodon.clone();
                return cosmic::task::future(async move {
                    Message::Compose(C::EditLoaded(
                        request,
                        compose::network::edit(client, id).await,
                    ))
                });
            }
            C::EditLoaded(request, result) => {
                if request != self.compose_request || self.active_compose().is_some() {
                    return Task::none();
                }
                return match result {
                    Ok(state) => self.open_compose(state),
                    Err(error) => cosmic::task::message(Message::Error(error)),
                };
            }
            C::Drafts => {
                if self.active_compose().is_some() {
                    return cosmic::task::message(Message::Error(fl!("compose-existing")));
                }
                self.load_drafts();
                self.dialog_pages.push_front(Dialog::Drafts);
            }
            C::DeleteDraft(id) => {
                if self.active_compose().is_some() {
                    return Task::none();
                }
                if let Err(error) = self.drop_draft(&id) {
                    return cosmic::task::message(Message::Error(error));
                }
            }
            C::Resume(id) => {
                if let Some(state) = self.drafts.iter().find(|draft| draft.id == id).cloned() {
                    return self.open_compose(state);
                }
            }
            C::Close => {
                let Some(state) = self.active_compose() else {
                    return Task::none();
                };
                if state.busy() {
                    return Task::none();
                }
                if state.published_id.is_some() {
                    self.close_compose_frames();
                    return Task::none();
                }
                if state.dirty || state.attempted_at.is_some() {
                    self.dialog_pages.push_front(Dialog::CloseDraft);
                } else if self.drop_active_draft() {
                    self.close_compose_frames();
                }
            }
            C::ContinueEditing => {
                if matches!(self.dialog_pages.front(), Some(Dialog::CloseDraft)) {
                    self.dialog_pages.pop_front();
                }
            }
            C::Keep => {
                if self.active_compose().is_some_and(|state| !state.busy()) {
                    self.dialog_pages
                        .retain(|dialog| !matches!(dialog, Dialog::CloseDraft));
                    if self.persist_active_draft() {
                        self.close_compose_frames();
                    }
                }
            }
            C::Discard => {
                if self.active_compose().is_some_and(|state| !state.busy()) {
                    self.dialog_pages
                        .retain(|dialog| !matches!(dialog, Dialog::CloseDraft));
                    if self.drop_active_draft() {
                        self.close_compose_frames();
                    }
                }
            }
            C::PickImages => {
                if self.active_compose().is_some_and(compose::State::editable) {
                    if let Some(state) = self.active_compose_mut() {
                        state.importing = true;
                    }
                    let request = self.compose_request;
                    return cosmic::task::future(async move {
                        Message::Compose(C::Picked(request, compose::network::pick_images().await))
                    });
                }
            }
            C::Picked(request, result) => {
                if request != self.compose_request {
                    return Task::none();
                }
                let Some(state) = self.active_compose() else {
                    return Task::none();
                };
                match result {
                    Ok(paths)
                        if !paths.is_empty()
                            && state.media.len() + paths.len()
                                <= self.compose_limits.attachments
                            && !state.has_poll =>
                    {
                        let store = self.compose_store.clone();
                        let key = self.mastodon.cache_key();
                        let id = state.id.clone();
                        let limits = self.compose_limits.clone();
                        return cosmic::task::future(async move {
                            let result = tokio::task::spawn_blocking(move || {
                                store.import(&key, &id, &paths, &limits)
                            })
                            .await
                            .unwrap_or_else(|_| Err(fl!("compose-file-error")));
                            Message::Compose(C::Imported(request, result))
                        });
                    }
                    Ok(paths) => {
                        if let Some(state) = self.active_compose_mut() {
                            state.importing = false;
                            if !paths.is_empty() {
                                state.error = Some(fl!("compose-too-many-images"));
                            }
                        }
                    }
                    Err(error) => {
                        if let Some(state) = self.active_compose_mut() {
                            state.importing = false;
                            state.error = Some(error);
                        }
                    }
                }
            }
            C::Imported(request, result) => {
                if request != self.compose_request {
                    return Task::none();
                }
                if let Some(state) = self.active_compose_mut() {
                    state.importing = false;
                    match result {
                        Ok(media) => {
                            let ids: Vec<_> = media.iter().map(|m| m.id.clone()).collect();
                            state.media.extend(media);
                            state.changed();
                            self.persist_active_draft();
                            return Task::batch(
                                ids.into_iter().map(|id| {
                                    cosmic::task::message(Message::Compose(C::Upload(id)))
                                }),
                            );
                        }
                        Err(error) => state.error = Some(error),
                    }
                }
                self.persist_active_draft();
            }
            C::Upload(id) => {
                let Some(state) = self.active_compose().filter(|state| state.accepts_edits())
                else {
                    return Task::none();
                };
                let Some(media) = state
                    .media
                    .iter()
                    .find(|media| media.id == id && media.stored_name.is_some())
                    .cloned()
                else {
                    return Task::none();
                };
                let Some(name) = &media.stored_name else {
                    return Task::none();
                };
                let Ok(path) =
                    self.compose_store
                        .asset_path(&self.mastodon.cache_key(), &state.id, name)
                else {
                    return Task::none();
                };
                if let Some(state) = self.active_compose_mut() {
                    if let Some(media) = state.media.iter_mut().find(|media| media.id == id) {
                        media.uploading = true;
                        media.attachment = None;
                        media.error = None;
                        media
                            .progress
                            .store(0, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                let client = self.mastodon.clone();
                let request = self.compose_request;
                return cosmic::task::future(async move {
                    Message::Compose(C::Uploaded(
                        request,
                        id,
                        compose::network::upload(client, path, media).await,
                    ))
                });
            }
            C::Uploaded(request, id, result) => {
                if request != self.compose_request {
                    return Task::none();
                }
                let mut previews = vec![];
                if let Some(state) = self.active_compose_mut() {
                    if let Some(media) = state.media.iter_mut().find(|media| media.id == id) {
                        media.uploading = false;
                        match result {
                            Ok(attachment) => {
                                if let Some(url) = &attachment.preview_url {
                                    previews.push(url.clone());
                                }
                                media.attachment = Some(attachment);
                                media.error = None;
                            }
                            Err(error) => media.error = Some(error),
                        }
                    }
                }
                self.persist_active_draft();
                return cosmic::task::message(Message::Fetch(previews));
            }
            C::Submit => {
                let Some(state) = self.active_compose() else {
                    return Task::none();
                };
                if state.busy() || !state.retry_safe() || !state.valid(&self.compose_limits) {
                    return Task::none();
                }
                if state.attempted_at.is_some() {
                    return self.begin_status_submission();
                }
                let state = state.clone();
                if let Some(current) = self.active_compose_mut() {
                    current.sending = true;
                    current.error = None;
                }
                let client = self.mastodon.clone();
                let request = self.compose_request;
                return cosmic::task::future(async move {
                    Message::Compose(C::Prepared(
                        request,
                        compose::network::prepare_media(client, state).await,
                    ))
                });
            }
            C::Prepared(request, result) => {
                if request != self.compose_request {
                    return Task::none();
                }
                if let Some(state) = self.active_compose_mut() {
                    state.sending = false;
                    match result {
                        Ok(attachments) => {
                            for attachment in attachments {
                                if let Some(media) = state.media.iter_mut().find(|media| {
                                    media
                                        .attachment
                                        .as_ref()
                                        .is_some_and(|current| current.id == attachment.id)
                                }) {
                                    media.attachment = Some(attachment);
                                }
                            }
                        }
                        Err(error) => {
                            state.error = Some(error);
                            self.persist_active_draft();
                            return Task::none();
                        }
                    }
                }
                return self.begin_status_submission();
            }
            C::Submitted(request, result) => {
                if request != self.compose_request {
                    return Task::none();
                }
                if self.active_compose().is_none() {
                    return Task::none();
                }
                match result {
                    Ok(published) => {
                        let (editing, reply) = self
                            .active_compose()
                            .map(|state| (state.edit_id.is_some(), state.in_reply_to_id.clone()))
                            .unwrap();
                        if let Some(state) = self.active_compose_mut() {
                            state.sending = false;
                            state.published_id = Some(published.id.clone());
                        }
                        if self.persist_active_draft() {
                            self.drop_active_draft();
                            self.close_compose_frames();
                        }
                        let mut tasks = vec![self
                            .toasts
                            .push(Toast::new(if editing {
                                fl!("compose-edited")
                            } else {
                                fl!("compose-posted")
                            }))
                            .map(cosmic::Action::App)];
                        if let Some(post) = published.post {
                            let images = crate::cache::extract_status_images(&post);
                            self.cache.insert_status(post.clone());
                            if !editing {
                                tasks.push(
                                    self.home
                                        .update(timeline::Message::PrependStatus(post.clone())),
                                );
                            }
                            if let Some(profile) = &mut self.profile {
                                if profile.account.id == post.account.id
                                    && !profile.posts.contains(&post.id)
                                {
                                    profile.posts.insert(0, post.id.clone());
                                }
                            }
                            if let Some(thread) = &mut self.thread {
                                if reply.as_ref() == Some(&thread.root)
                                    && !thread.descendants.contains(&post.id)
                                {
                                    thread.descendants.push(post.id.clone());
                                }
                            }
                            tasks.push(cosmic::task::message(Message::Fetch(images)));
                        }
                        return Task::batch(tasks);
                    }
                    Err(error) => {
                        if let Some(state) = self.active_compose_mut() {
                            state.failed(error);
                        }
                        self.persist_active_draft();
                    }
                }
            }
            C::AllowEditing => {
                if let Some(state) = self.active_compose_mut() {
                    if !state.busy() && !state.retry_safe() {
                        state.attempted_at = None;
                        state.changed();
                    }
                }
                self.persist_active_draft();
            }
            C::Pane(entity) => self.compose_tabs.activate(entity),
            C::EditAlt(id) => {
                if self.active_compose().is_some_and(|state| {
                    state.accepts_edits() && state.media.iter().any(|m| m.id == id)
                }) {
                    self.dialog_pages.push_front(Dialog::AltText(id));
                    return widget::text_input::focus(compose::alt_input_id());
                }
            }
            C::Alt(id, value) => {
                if let Some(state) = self.active_compose_mut().filter(|s| s.accepts_edits()) {
                    if let Some(media) = state.media.iter_mut().find(|media| media.id == id) {
                        media.description = value;
                    }
                    state.changed();
                }
                self.persist_active_draft();
            }
            C::Tick => (),
            change => {
                if let Some(state) = self.active_compose_mut().filter(|state| state.editable()) {
                    match change {
                        C::Warning(value) => state.content_warning = value,
                        C::Spoiler(value) => state.spoiler_text = value,
                        C::Sensitive(value) => state.sensitive = value,
                        C::Visibility(value) => {
                            if state.edit_id.is_none()
                                && state.reply_ceiling.as_ref().is_none_or(|ceiling| {
                                    compose::visibility_rank(&value)
                                        >= compose::visibility_rank(ceiling)
                                })
                            {
                                state.visibility = value;
                            }
                        }
                        C::Language(value) => {
                            state.language =
                                (!value.trim().is_empty()).then(|| value.trim().to_string())
                        }
                        C::Remove(id) => state.media.retain(|media| media.id != id),
                        _ => return Task::none(),
                    }
                    state.changed();
                }
                self.persist_active_draft();
            }
        }
        Task::none()
    }

    fn apply_onboarding_effect(&mut self, effect: onboarding::Effect) -> Task<Message> {
        use onboarding::{Effect, Failure};
        match effect {
            Effect::None => Task::none(),
            Effect::Register(request, server) => cosmic::task::future(async move {
                Message::Onboarding(onboarding::register(request, server).await)
            }),
            Effect::Exchange(request, server, registration, code) => {
                cosmic::task::future(async move {
                    Message::Onboarding(
                        onboarding::exchange(request, server, registration, code).await,
                    )
                })
            }
            Effect::Verify(request, client) => cosmic::task::future(async move {
                Message::Onboarding(onboarding::verify(request, client).await)
            }),
            Effect::OpenBrowser(url) => {
                if open::that_detached(url).is_err() {
                    cosmic::task::message(Message::Onboarding(onboarding::Message::BrowserFailed))
                } else {
                    Task::none()
                }
            }
            Effect::Save(request, mut client, account) => {
                client.account_id = account.id.clone();
                let result = self.save_setup_account(&client, &account);
                let mut tasks = vec![cosmic::task::message(Message::Onboarding(
                    onboarding::Message::Saved(request, result),
                ))];
                if result.is_ok() {
                    tasks.push(cosmic::task::message(Message::Fetch(vec![account.avatar])));
                }
                Task::batch(tasks)
            }
            Effect::Finish(selected) => {
                // Persist before dismissing setup, so guest browsing survives restart.
                let saved = self.handler.as_ref().is_some_and(|handler| {
                    self.config.set_onboarding_completed(handler, true).is_ok()
                });
                if !saved {
                    if let Some(state) = &mut self.onboarding {
                        state.error = Some(Failure::Preferences);
                    }
                    return Task::none();
                }
                self.config.onboarding_completed = true;
                self.onboarding = None;
                match selected {
                    Some(index) => self.switch_account(index),
                    None => self.update_title(),
                }
            }
            Effect::Close => {
                self.onboarding = None;
                self.update_title()
            }
        }
    }

    fn save_setup_account(
        &mut self,
        client: &Client,
        account: &Account,
    ) -> Result<usize, onboarding::Failure> {
        use onboarding::Failure;
        // If startup could not read the keychain, reload it before merging.
        // Unlocking it must never replace an existing account list with an empty one.
        let mut sessions = if self.sessions_loaded {
            self.sessions.clone()
        } else {
            match keytar::get_password(Self::APP_ID, "mastodon-data") {
                Ok(data) if data.success => {
                    Sessions::parse(&data.password).ok_or(Failure::Keychain)?
                }
                Ok(_) => Sessions::default(),
                Err(_) => return Err(Failure::Keychain),
            }
        };
        let active = sessions.active;
        let preferences = sessions
            .sessions
            .iter()
            .find(|session| session.base_url == client.base_url && session.account_id == account.id)
            .and_then(|session| session.preferences.clone());
        sessions.upsert_active(Session {
            base_url: client.base_url.clone(),
            token: client.token.clone().unwrap_or_default(),
            account_id: account.id.clone(),
            acct: account.acct.clone(),
            preferences,
        });
        let selected = sessions.active;
        // Adding accounts does not change the client being used behind the wizard.
        if self.mastodon.is_authenticated() {
            sessions.active = active;
        }
        let data = serde_json::to_string(&sessions).map_err(|_| Failure::Keychain)?;
        keytar::set_password(Self::APP_ID, "mastodon-data", &data)
            .map_err(|_| Failure::Keychain)?;
        self.sessions = sessions;
        self.sessions_loaded = true;
        Ok(selected)
    }

    /// Persist the full account list (and which one is active) to the keychain.
    fn persist_sessions(&self) -> Result<(), String> {
        let data = serde_json::to_string(&self.sessions).map_err(|err| err.to_string())?;
        keytar::set_password(Self::APP_ID, "mastodon-data", &data).map_err(|err| err.to_string())
    }

    /// Drop every feed's in-memory content and reload each from the newly
    /// active account's own disk cache, so switching accounts doesn't leave
    /// the previous account's posts visible until a fresh fetch lands.
    fn reset_and_reload_feeds(&mut self) -> Task<Message> {
        self.generation += 1;
        self.filters_loading = false;
        self.filter_request += 1;
        self.thread = None;
        self.profile = None;
        self.context_history.clear();
        self.cache.reading = reading::State::new(self.mastodon.is_authenticated());
        self.dialog_pages.clear();
        self.dialog_editor = widget::text_editor::Content::default();
        self.core.window.show_context = false;
        self.context_page = ContextPage::default();
        self.image_queue.clear();
        self.image_inflight.clear();
        self.compose_limits = compose::Limits::default();
        self.load_drafts();
        let mastodon = self.mastodon.clone();
        self.hashtags = hashtags::Hashtags::new(mastodon.clone());
        self.lists = lists::Lists::new(mastodon.clone());
        self.search = search::Search::new(mastodon.clone());
        self.home.reset(mastodon.clone());
        self.explore.reset(mastodon.clone());
        self.local.reset(mastodon.clone());
        self.federated.reset(mastodon.clone());
        self.favorites.reset(mastodon.clone());
        self.bookmarks.reset(mastodon.clone());
        self.notifications.reset(mastodon.clone());
        let load_tasks = vec![
            self.home.load_cached(),
            self.explore.load_cached(),
            self.local.load_cached(),
            self.federated.load_cached(),
            self.favorites.load_cached(),
            self.bookmarks.load_cached(),
            self.notifications.load_cached(),
        ];
        Task::batch(vec![
            self.hashtags
                .update(hashtags::Message::SetClient(mastodon.clone())),
            self.lists
                .update(lists::Message::SetClient(mastodon.clone())),
            self.search.update(search::Message::SetClient(mastodon)),
            Task::batch(load_tasks),
            self.reload_filters(),
        ])
    }

    /// Start image downloads for as many queued URLs as fit under
    /// [`MAX_CONCURRENT_IMAGE_FETCHES`], taking from the front of the queue
    /// first (i.e. whichever were requested earliest — newest posts, since
    /// they're rendered and their images requested before older ones).
    fn drain_image_queue(&mut self) -> Task<Message> {
        let mut tasks = vec![];
        while self.image_inflight.len() < MAX_CONCURRENT_IMAGE_FETCHES {
            let Some(url) = self.image_queue.pop_front() else {
                break;
            };
            self.image_inflight.insert(url.clone());
            tasks.push(cosmic::task::future(async move {
                match crate::cache::get(&url).await {
                    Ok(handle) => Message::CacheHandle(url, handle),
                    Err(err) => {
                        tracing::error!("Failed to fetch image: {}", err);
                        Message::ImageFetchFailed(url)
                    }
                }
            }));
        }
        Task::batch(tasks)
    }

    /// Save every feed's cached content to disk, if anything changed since
    /// the last flush.
    fn flush_cache_to_disk(&mut self) {
        if !self.cache.dirty {
            return;
        }
        self.home.save_cached(&self.cache);
        self.explore.save_cached(&self.cache);
        self.local.save_cached(&self.cache);
        self.federated.save_cached(&self.cache);
        self.favorites.save_cached(&self.cache);
        self.bookmarks.save_cached(&self.cache);
        self.notifications.save_cached(&self.cache);
        self.cache.dirty = false;
    }

    /// Switch to the saved account at `index`, resetting per-account state.
    fn switch_account(&mut self, index: usize) -> Task<Message> {
        if self.active_compose().is_some_and(compose::State::busy) {
            return cosmic::task::message(Message::Error(fl!("compose-busy")));
        }
        if !self.persist_active_draft() {
            return Task::none();
        }
        let Some(session) = self.sessions.sessions.get(index).cloned() else {
            return Task::none();
        };
        self.flush_cache_to_disk();
        if self.mastodon.is_authenticated() {
            if let Some(current) = self.sessions.sessions.get_mut(self.sessions.active) {
                current.preferences = Some(self.config.clone());
            }
        }
        self.sessions.active = index;
        let onboarding_completed = self.config.onboarding_completed;
        self.config = session.preferences.clone().unwrap_or_else(|| TootConfig {
            server: self.config.server.clone(),
            ..Default::default()
        });
        self.config.onboarding_completed = onboarding_completed;
        self.instance = session.base_url.clone();
        self.mastodon = Client::from_session(&session);
        self.cache.clear();
        self.cache.hide_boosts = self.config.hide_boosts;
        self.cache.hide_replies = self.config.hide_replies;
        self.cache.feed_density = self.config.feed_density;
        self.update_navbar();

        let mut tasks = vec![
            self.reset_and_reload_feeds(),
            self.on_nav_select(self.nav.active()),
        ];
        if let Err(err) = self.persist_sessions() {
            tasks.push(cosmic::task::message(Message::Error(format!(
                "Couldn't save session: {err}"
            ))));
        }
        tasks.push(fetch_session_info(self.mastodon.clone()));
        Task::batch(tasks)
    }

    /// Remove the saved account at `index`. If it was active, switches to
    /// another remaining account or falls back to a logged-out client.
    fn remove_account(&mut self, index: usize) -> Task<Message> {
        if index == self.sessions.active {
            if self.active_compose().is_some_and(compose::State::busy) {
                return cosmic::task::message(Message::Error(fl!("compose-busy")));
            }
            if !self.persist_active_draft() {
                return Task::none();
            }
        }
        let was_active = index == self.sessions.active;
        let removed_base_url = self
            .sessions
            .sessions
            .get(index)
            .map(|s| Client::from_session(s).cache_key());
        if was_active {
            self.flush_cache_to_disk();
        }
        self.sessions.remove(index);
        if let Some(base_url) = removed_base_url {
            crate::persistence::clear_account(&base_url);
        }

        let mut tasks = vec![];
        if let Err(err) = self.persist_sessions() {
            tasks.push(cosmic::task::message(Message::Error(format!(
                "Couldn't save session: {err}"
            ))));
        }

        if was_active {
            match self.sessions.active_session().cloned() {
                Some(session) => {
                    self.mastodon = Client::from_session(&session);
                }
                None => {
                    self.mastodon = Client::new(self.instance(), None);
                }
            }
            self.cache.clear();
            self.cache.hide_boosts = self.config.hide_boosts;
            self.cache.hide_replies = self.config.hide_replies;
            self.cache.feed_density = self.config.feed_density;
            self.update_navbar();
            tasks.push(self.reset_and_reload_feeds());
            tasks.push(self.on_nav_select(self.nav.active()));
        }
        Task::batch(tasks)
    }

    pub fn update_title(&mut self) -> Task<Message> {
        let mut window_title = fl!("app-title");
        if let Some(page) = self.nav.text(self.nav.active()) {
            window_title.push_str(" — ");
            window_title.push_str(page);
        }
        if let Some(id) = self.core.main_window_id() {
            self.set_window_title(window_title, id)
        } else {
            Task::none()
        }
    }

    fn switch_instance(&self, instance: String) -> widget::Dialog<'_, Message> {
        widget::dialog()
            .title(fl!("server-question"))
            .body(fl!("server-description"))
            .icon(widget::icon::from_name("network-server-symbolic"))
            .control(
                widget::text_input(fl!("server-url"), instance)
                    .on_input(|instance| {
                        Message::Dialog(DialogAction::Update(Dialog::SwitchInstance(instance)))
                    })
                    .on_submit(|_| Message::Dialog(DialogAction::Complete)),
            )
            .primary_action(
                widget::button::suggested(fl!("confirm"))
                    .on_press(Message::Dialog(DialogAction::Complete)),
            )
            .secondary_action(
                widget::button::standard(fl!("cancel"))
                    .on_press(Message::Dialog(DialogAction::Close)),
            )
    }

    fn logout(&self) -> widget::Dialog<'_, Message> {
        widget::dialog()
            .title(fl!("logout-question"))
            .body(fl!("logout-description"))
            .icon(widget::icon::from_name("system-log-out-symbolic"))
            .primary_action(
                widget::button::suggested(fl!("continue"))
                    .on_press(Message::Dialog(DialogAction::Complete)),
            )
            .secondary_action(
                widget::button::standard(fl!("cancel"))
                    .on_press(Message::Dialog(DialogAction::Close)),
            )
    }

    fn delete_status(&self, _id: String) -> widget::Dialog<'_, Message> {
        widget::dialog()
            .title(fl!("delete-status-question"))
            .body(fl!("delete-status-description"))
            .icon(widget::icon::from_name("user-trash-symbolic"))
            .primary_action(
                widget::button::destructive(fl!("delete"))
                    .on_press(Message::Dialog(DialogAction::Complete)),
            )
            .secondary_action(
                widget::button::standard(fl!("cancel"))
                    .on_press(Message::Dialog(DialogAction::Close)),
            )
    }

    /// A full-window image overlay, in the same spirit as cosmic-files'
    /// gallery view: a translucent backdrop covering the whole window with
    /// the image centered and content-fit, rather than a small dialog box.
    fn view_image(&self, preview_url: String, original_url: String) -> Element<'_, Message> {
        let spacing = cosmic::theme::active().cosmic().spacing;

        let image: Element<'_, Message> = self
            .cache
            .handles
            .get(&preview_url)
            .map(|handle| {
                widget::image(handle)
                    .content_fit(cosmic::iced::ContentFit::Contain)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            })
            .unwrap_or_else(|| widget::text("Image not available").into());

        // Fixed, matching top/bottom bars so the image's `Length::Fill` area
        // in between is symmetric — an empty bottom bar mirrors the header's
        // height rather than letting the image run flush to the window edge.
        // The fixed height is on the *wrapping container*, not the row itself,
        // so the buttons are centered within it rather than being squashed
        // to fit. Generous headroom above the row's own intrinsic size
        // (buttons + padding) so the container never has to compress it —
        // a too-tight fixed height here is what squashed the buttons before.
        let bar_height = Length::Fixed(96.0);

        let header_row = widget::row![
            widget::space::horizontal(),
            widget::button::icon(widget::icon::from_name("send-to-symbolic"))
                .class(cosmic::style::Button::Standard)
                .on_press(Message::Open(original_url)),
            widget::button::icon(widget::icon::from_name("window-close-symbolic"))
                .class(cosmic::style::Button::Standard)
                .on_press(Message::Dialog(DialogAction::Close)),
        ]
        .align_y(cosmic::iced::Alignment::Center)
        .spacing(spacing.space_xs)
        .padding(spacing.space_xs);

        let header = widget::container(header_row)
            .width(Length::Fill)
            .height(bar_height)
            .align_y(Vertical::Center);

        let footer = widget::container(widget::text(""))
            .width(Length::Fill)
            .height(bar_height);

        widget::column![
            header,
            widget::container(image).center(Length::Fill),
            footer
        ]
        .apply(widget::container)
        .width(Length::Fill)
        .height(Length::Fill)
        .class(cosmic::style::Container::Dialog(false))
        .into()
    }

    fn reload_filters(&mut self) -> Task<Message> {
        if !self.mastodon.is_authenticated() || self.filters_loading {
            return Task::none();
        }
        self.filters_loading = true;
        if matches!(
            self.cache.reading.filters,
            reading::filters::Filters::Failed
        ) {
            self.cache.reading.filters = reading::filters::Filters::Loading;
        }
        self.filter_request += 1;
        let request = self.filter_request;
        let client = self.mastodon.clone();
        cosmic::task::future(async move {
            Message::FiltersLoaded(request, reading::filters::fetch(client).await)
        })
    }

    fn find_poll(&self, id: &str) -> Option<&megalodon::entities::Poll> {
        self.cache.reading.polls.get(id).or_else(|| {
            self.cache
                .statuses
                .values()
                .filter_map(|post| post.poll.as_ref())
                .find(|poll| poll.id == id)
        })
    }

    fn load_reader(&mut self) -> Task<Message> {
        self.reader_request += 1;
        let request = self.reader_request;
        self.thread = None;
        self.profile = None;
        match self.context_page.clone() {
            ContextPage::Status(id) => {
                self.thread = Some(reading::thread::Thread::new(request, id.clone()));
                let client = self.mastodon.clone();
                cosmic::task::future(async move {
                    Message::ThreadLoaded(request, reading::thread::fetch(client, id).await)
                })
            }
            ContextPage::Account(account) => {
                let images = vec![account.avatar.clone(), account.header.clone()];
                let id = account.id.clone();
                self.profile = Some(reading::profile::Profile::new(request, account));
                let client = self.mastodon.clone();
                Task::batch(vec![
                    cosmic::task::message(Message::Fetch(images)),
                    cosmic::task::future(async move {
                        Message::ProfileLoaded(
                            request,
                            reading::profile::fetch(client, id, None).await,
                        )
                    }),
                ])
            }
            _ => Task::none(),
        }
    }

    fn reader_view<'a>(&'a self, content: Element<'a, Message>) -> Element<'a, Message> {
        let mut view = widget::column![].spacing(12);
        if !self.context_history.is_empty() {
            view = view
                .push(widget::button::standard(fl!("reading-back")).on_press(Message::BackContext));
        }
        view.push(content).into()
    }

    fn status(&self, _id: &String) -> Element<'_, Message> {
        self.thread
            .as_ref()
            .map(|thread| thread.view(&self.cache))
            .unwrap_or_else(|| widget::text(fl!("reading-unavailable")).into())
    }

    fn account<'a>(&'a self, account: &'a Account) -> Element<'a, Message> {
        accounts::account(account, &self.cache).map(Message::Account)
    }
}

/// Fetch the authenticated account and the instance's status length limit,
/// used to gate the compose dialog's delete action and character counter.
fn fetch_session_info(mastodon: Client) -> Task<Message> {
    let account_client = mastodon.clone();
    let instance_client = mastodon;
    Task::batch(vec![
        cosmic::task::future(async move {
            match account_client.verify_account_credentials().await {
                Ok(response) => Message::SetAccount(response.json),
                Err(err) => Message::Error(format!("Couldn't load account: {err}")),
            }
        }),
        cosmic::task::future(async move {
            match compose::network::limits(instance_client).await {
                Ok(limits) => Message::SetComposeLimits(limits),
                Err(error) => Message::Error(error),
            }
        }),
    ])
}

fn instance(instance: impl Into<String>) -> String {
    let instance: String = instance.into();
    let instance = instance
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if instance.is_empty() {
        format!("https://{}", "mastodon.social")
    } else {
        format!("https://{}", instance)
    }
}

impl AppModel
where
    Self: Application,
{
    fn instance(&self) -> String {
        instance(self.instance.clone())
    }

    fn update_navbar(&mut self) {
        self.nav.clear();

        let variants = (!self.mastodon.is_authenticated())
            .then(Page::public_variants)
            .unwrap_or_else(Page::variants);

        for page in variants {
            let id = self
                .nav
                .insert()
                .text(page.to_string())
                .icon(widget::icon(page.icon()))
                .data::<Page>(page.clone())
                .id();
            if self.mastodon.is_authenticated() && matches!(page, Page::Favorites | Page::Explore) {
                self.nav.divider_above_set(id, true);
            }
        }
        self.nav.activate_position(0);
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum ContextPage {
    #[default]
    About,
    Account(Account),
    Status(String),
    Settings,
}

impl ContextPage {
    fn title(&self) -> String {
        match self {
            ContextPage::About => fl!("about"),
            ContextPage::Account(_) => fl!("profile"),
            ContextPage::Status(_) => fl!("status"),
            ContextPage::Settings => "Settings".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    Drafts,
    Logout,
    About,
    Settings,
}

impl menu::action::MenuAction for MenuAction {
    type Message = Message;

    fn message(&self) -> Self::Message {
        match self {
            MenuAction::Drafts => Message::Compose(compose::Message::Drafts),
            MenuAction::Logout => Message::Dialog(DialogAction::Open(Dialog::Logout)),
            MenuAction::About => Message::ToggleContextPage(ContextPage::About),
            MenuAction::Settings => Message::ToggleContextPage(ContextPage::Settings),
        }
    }
}

fn scoped_task(task: Task<Message>, generation: u64) -> Task<Message> {
    task.map(move |action| match action {
        cosmic::Action::App(message) => {
            cosmic::Action::App(Message::Scoped(generation, Box::new(message)))
        }
        other => other,
    })
}
