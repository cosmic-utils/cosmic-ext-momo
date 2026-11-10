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
    stream_state: crate::streaming::State,
    session_expired: bool,
    confirm_clear_cache: bool,
    clearing_cache: bool,
    generation: u64,
    activity: notifications::desktop::Activity,
    social: crate::features::social::Social,
    video: Option<crate::media::Player>,
    video_url: Option<String>,
    gallery_image_url: Option<String>,
    video_failed: bool,
    reader_request: u64,
    filter_request: u64,
    filters_loading: bool,
    main_page: Option<ContextPage>,
    context_history: Vec<ContextPage>,
    thread: Option<reading::thread::Thread>,
    media_thread: Option<reading::thread::Thread>,
    media_details: bool,
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
    draft_revision: u64,
    draft_save_due: Option<std::time::Instant>,
    draft_saving: bool,
    cache_saving: Option<u64>,
    compose_request: u64,
    completion_serial: u64,
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
    StreamState(crate::streaming::State),
    SessionExpired,
    CacheCleared(Result<(), String>),
    StatusUpdated(Status),
    BackContext,
    RetryReader,
    ToggleReplies(String),
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
    MyProfile,
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
    Social(crate::features::social::Message),
    Account(accounts::Message),
    Status(status::Message),
    Fetch(Vec<String>),
    CacheStatus(Status),
    FavoriteChanged(Status),
    BookmarkChanged(Status),
    CacheNotification(Notification),
    RemoveNotifications(Vec<String>),
    CacheRelationship(megalodon::entities::Relationship),
    CacheRelationships(Vec<megalodon::entities::Relationship>),
    CacheHandle(u64, String, Handle),
    /// An image download failed; drop it from the in-flight set and let the
    /// next queued one start.
    ImageFetchFailed(u64, String),
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
    Surface(cosmic::surface::Action<Message>),
    NavigateMedia(bool),
    ToggleMediaDetails,
    RetryMediaThread,
    MediaStatus(Box<status::Message>),
    VideoTick,
    VideoFrameUploaded(
        cosmic::iced::core::image::Id,
        Result<cosmic::iced::runtime::image::Allocation, cosmic::iced::runtime::image::Error>,
    ),
    VideoPause,
    VideoMute,
    VideoSeek(f64),
    VideoSeekReleased,
    /// Periodic tick: flush every feed's cache to disk if anything changed.
    FlushCache,
    SaveDraft,
    DraftSaved(u64, Result<(), String>),
    CacheSaved(u64, Result<(), String>),
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
    EditLoading {
        id: String,
        request: u64,
        error: Option<String>,
    },
    CloseDraft,
    /// Edit the alt text of a compose attachment, by media id.
    AltText(String),
    Report(reading::report::State),
    Compose(compose::State),
    SwitchInstance(String),
    Logout,
    DeleteStatus(String),
    /// Post images and videos, with the selected index and originating status ID.
    Media(Vec<crate::media::Attachment>, usize, String),
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
            .version("0.1.3")
            .icon(widget::icon::from_name(Self::APP_ID))
            .author("Eduardo Flores")
            .developers([("Eduardo Flores", "edfloreshz@proton.me")])
            .links([(fl!("repository"), REPOSITORY), (fl!("support"), SUPPORT)]);

        let mut app = AppModel {
            generation: 0,
            video: None,
            video_url: None,
            gallery_image_url: None,
            video_failed: false,
            stream_state: crate::streaming::State::Connecting,
            session_expired: false,
            confirm_clear_cache: false,
            clearing_cache: false,
            reader_request: 0,
            media_thread: None,
            media_details: true,
            filter_request: 0,
            filters_loading: false,
            main_page: None,
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
            social: crate::features::social::Social::default(),
            activity: notifications::desktop::Activity::load(&mastodon.cache_key()),
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
            draft_revision: 0,
            draft_save_due: None,
            draft_saving: false,
            cache_saving: None,
            compose_request: 0,
            completion_serial: 0,
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
        app.update_activity_badge();

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
                fl!("my-profile"),
                Some(widget::icon::from_name("avatar-default-symbolic").into()),
                MenuAction::MyProfile,
            ));
            items.push(menu::Item::Button(
                fl!("private-conversations"),
                Some(widget::icon::from_name("mail-message-new-symbolic").into()),
                MenuAction::Conversations,
            ));
            items.push(menu::Item::Button(
                fl!("scheduled-posts"),
                Some(widget::icon::from_name("appointment-soon-symbolic").into()),
                MenuAction::Scheduled,
            ));
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
        .window_id_maybe(self.core.main_window_id())
        .on_surface_action(Message::Surface)
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
            vec![
                widget::button::standard(fl!("guest-change-server"))
                    .on_press(Message::Dialog(DialogAction::Open(Dialog::SwitchInstance(
                        self.mastodon.base_url.clone(),
                    ))))
                    .into(),
                widget::button::standard(fl!("setup-add-account"))
                    .on_press(Message::StartOnboarding)
                    .into(),
            ]
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
        self.main_page = None;
        self.context_history.clear();
        self.nav.activate(id);
        if self.nav.data::<Page>(id) == Some(&Page::Notifications) {
            self.activity.read();
            self.activity.save(&self.mastodon.cache_key());
            self.update_activity_badge();
        }
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
            ContextPage::Social(crate::features::social::Page::People(_, _)) => {
                context_drawer::context_drawer(
                    self.social.view(&self.cache),
                    Message::ToggleContextDrawer,
                )
                .title(self.context_page.title())
            }
            ContextPage::Account(_) | ContextPage::Status(_) | ContextPage::Social(_) => {
                return None;
            }
            ContextPage::Settings => {
                let content = settings::view(
                    &self.config,
                    &self.sessions.sessions,
                    self.sessions.active,
                    self.confirm_clear_cache,
                    self.clearing_cache,
                )
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
        // The media viewer is a full-window overlay (like cosmic-files' gallery
        // view), not a small centered dialog box, so it's built and returned
        // before the `widget::dialog()`-based match below.
        if let Some(Dialog::Media(images, index, _)) = self.dialog_pages.front() {
            return Some(self.view_media(images, *index));
        }

        if self.dialog_pages.is_empty() && self.nav.active_data::<Page>() == Some(&Page::Lists) {
            if let Some(dialog) = self.lists.dialog(&self.cache) {
                return Some(dialog);
            }
        }
        let dialog_page = self.dialog_pages.front()?;
        if let Dialog::Report(state) = dialog_page {
            return Some(state.view());
        }

        let dialog = match dialog_page {
            Dialog::EditLoading { id, error, .. } => {
                let mut content = widget::column![]
                    .spacing(16)
                    .align_x(cosmic::iced::Alignment::Center);
                if let Some(error) = error {
                    content = content.push(widget::text(error.clone()));
                } else {
                    content = content
                        .push(widget::indeterminate_circular().size(32.0))
                        .push(widget::text(fl!("compose-loading-edit")));
                }
                let dialog = widget::dialog()
                    .title(fl!("compose-edit"))
                    .control(content)
                    .secondary_action(
                        widget::button::standard(fl!("cancel"))
                            .on_press(Message::Dialog(DialogAction::Close)),
                    );
                if error.is_some() {
                    dialog.primary_action(
                        widget::button::suggested(fl!("compose-retry"))
                            .on_press(Message::Compose(compose::Message::Edit(id.clone()))),
                    )
                } else {
                    dialog
                }
            }
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
                    &self.cache,
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
                    &self.cache,
                ),
                None => return None,
            },
            Dialog::Logout => self.logout(),
            Dialog::DeleteStatus(id) => self.delete_status(id.clone()),
            Dialog::Media(..) | Dialog::Report(_) => unreachable!("handled above"),
        };

        Some(dialog.into())
    }

    fn on_escape(&mut self) -> Task<Self::Message> {
        if self.onboarding.is_some() {
            return Task::none();
        }
        if self.nav.active_data::<Page>() == Some(&Page::Lists) && self.lists.has_menu() {
            return self.update(Message::Lists(lists::Message::Menu(None)));
        }
        if self.dialog_pages.is_empty()
            && self.nav.active_data::<Page>() == Some(&Page::Lists)
            && self.lists.is_editing()
        {
            return self.update(Message::Lists(lists::Message::Editor(
                lists::editor::Message::Close,
            )));
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
        let content = if let Some(page) = &self.main_page {
            let content = match page {
                ContextPage::Account(account) => self
                    .profile
                    .as_ref()
                    .map(|profile| profile.view(&self.cache))
                    .unwrap_or_else(|| self.account(account)),
                ContextPage::Status(id) => self.status(id),
                ContextPage::Social(_) => self.social.view(&self.cache),
                _ => unreachable!("drawer page in main view"),
            };
            self.reader_view(content)
        } else {
            match self.nav.active_data::<Page>() {
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
                None => widget::text(fl!("select-page")).into(),
            }
        }
        .apply(widget::container)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center);

        let mut body = widget::column![].spacing(8);
        if self.session_expired {
            body = body.push(
                widget::column![
                    widget::text(fl!("session-expired")),
                    widget::button::suggested(fl!("session-sign-in"))
                        .on_press(Message::StartOnboarding)
                ]
                .spacing(12)
                .padding(12),
            );
        } else if self.mastodon.is_authenticated()
            && self.stream_state == crate::streaming::State::Reconnecting
        {
            body = body.push(widget::text::caption(fl!("stream-reconnecting")));
        }
        body = body.push(content);
        widget::toaster(&self.toasts, body)
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let mut subscriptions = vec![self
            .core()
            .watch_config::<TootConfig>(Self::APP_ID)
            .map(|update| Message::UpdateConfig(update.config))];

        if self.onboarding.is_some() {
            return Subscription::batch(subscriptions);
        }

        if self.cache.reading.post_menu.is_some() {
            subscriptions.push(cosmic::iced::event::listen_with(|event, _, _| {
                matches!(
                    event,
                    cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyPressed {
                        key: cosmic::iced::keyboard::Key::Named(
                            cosmic::iced::keyboard::key::Named::Escape
                        ),
                        ..
                    })
                )
                .then_some(Message::Status(status::Message::Menu(None)))
            }));
        }
        if self.video.is_some() {
            subscriptions.push(
                cosmic::iced::time::every(std::time::Duration::from_millis(33))
                    .map(|_| Message::VideoTick),
            );
        }
        if matches!(self.dialog_pages.front(), Some(Dialog::Media(..))) {
            subscriptions.push(cosmic::iced::event::listen_with(|event, _, _| {
                let cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyPressed {
                    key,
                    ..
                }) = event
                else {
                    return None;
                };
                use cosmic::iced::keyboard::{key::Named, Key};
                match key {
                    Key::Named(Named::ArrowLeft) => Some(Message::NavigateMedia(false)),
                    Key::Named(Named::ArrowRight) => Some(Message::NavigateMedia(true)),
                    Key::Character(ref text) if text == " " => Some(Message::VideoPause),
                    Key::Named(Named::Escape) => Some(Message::Dialog(DialogAction::Close)),
                    _ => None,
                }
            }));
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
            if !self.session_expired {
                subscriptions.push(crate::streaming::stream_user_events(self.mastodon.clone()));
            }
        }

        subscriptions.push(
            cosmic::iced::time::every(std::time::Duration::from_secs(30))
                .map(|_| Message::FlushCache),
        );

        if self.draft_save_due.is_some() {
            subscriptions.push(
                cosmic::iced::time::every(std::time::Duration::from_millis(250))
                    .map(|_| Message::SaveDraft),
            );
        }

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
            Message::Surface(action) => {
                return cosmic::task::message(cosmic::Action::Surface(action))
            }
            Message::Scoped(generation, message) => {
                if generation != self.generation {
                    return Task::none();
                }
                return self.update(*message);
            }
            Message::SessionExpired => self.session_expired = true,
            Message::StreamState(state) => {
                let previous = self.stream_state;
                self.stream_state = state;
                if state == crate::streaming::State::Unauthorized {
                    tasks.push(self.update(Message::SessionExpired));
                } else if state == crate::streaming::State::Connected
                    && previous != crate::streaming::State::Connected
                {
                    tasks.push(self.home.update(timeline::Message::Recover));
                    tasks.push(self.notifications.update(notifications::Message::Recover));
                    tasks.push(self.reload_filters());
                }
            }
            Message::CacheCleared(result) => {
                self.clearing_cache = false;
                self.confirm_clear_cache = false;
                self.gallery_image_url = None;
                self.cache.handles.clear();
                self.image_queue.clear();
                self.image_inflight.clear();
                self.cache.dirty = false;
                tasks.push(self.update(Message::Error(match result {
                    Ok(()) => fl!("settings-cache-cleared"),
                    Err(error) => error,
                })));
            }
            Message::StatusUpdated(status) => {
                if !self.cache.deleted.contains(&status.id) {
                    // An edit can change content warnings, media sensitivity, and filter matches.
                    self.cache.reading.warnings.remove(&status.id);
                    self.cache.reading.media.remove(&status.id);
                    self.cache
                        .reading
                        .filter_reveals
                        .retain(|(id, _)| id != &status.id);
                    tasks.push(
                        self.update(Message::Fetch(crate::cache::extract_status_images(&status))),
                    );
                    self.cache.insert_status(status);
                }
            }
            Message::BackContext => {
                if let Some(page) = self.context_history.pop() {
                    if let ContextPage::Social(social) = &page {
                        tasks.push(self.social.open(social.clone(), self.mastodon.clone()));
                    }
                    self.main_page = Some(page);
                    tasks.push(self.load_reader());
                } else {
                    self.main_page = None;
                }
                tasks.push(self.update_title());
            }
            Message::RetryReader => tasks.push(self.load_reader()),
            Message::ToggleReplies(id) => {
                if !self.cache.reading.collapsed_replies.remove(&id) {
                    self.cache.reading.collapsed_replies.insert(id);
                }
            }
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
                if let Some(thread) = self
                    .thread
                    .iter_mut()
                    .chain(self.media_thread.iter_mut())
                    .find(|thread| thread.request == request)
                {
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
                if let notifications::Message::PageLoaded(request, Ok(page)) = &message {
                    if self.notifications.is_refresh_request(*request) {
                        self.activity.reconcile(
                            page.notifications
                                .iter()
                                .map(|notification| notification.id.clone()),
                        );
                        if self.nav.active_data::<Page>() == Some(&Page::Notifications) {
                            self.activity.read();
                        }
                        self.activity.save(&self.mastodon.cache_key());
                        self.update_activity_badge();
                    }
                }
                if let notifications::Message::PrependNotification(notification) = &message {
                    if self.activity.record(&notification.id) {
                        self.activity.save(&self.mastodon.cache_key());
                        self.update_activity_badge();
                        if !self.config.disable_desktop_notifications {
                            let summary = notifications::desktop::summary(notification);
                            tasks.push(cosmic::task::future(async move {
                                if !matches!(
                                    tokio::time::timeout(
                                        std::time::Duration::from_secs(5),
                                        notifications::desktop::show(summary)
                                    )
                                    .await,
                                    Ok(Ok(()))
                                ) {
                                    tracing::warn!("desktop notification service is unavailable");
                                }
                                Message::None
                            }));
                        }
                    }
                }
                if matches!(message, notifications::Message::PublishNew) {
                    self.activity.read();
                    self.activity.save(&self.mastodon.cache_key());
                    self.update_activity_badge();
                }
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
                if matches!(
                    &message,
                    hashtags::Message::Select(_) | hashtags::Message::Deselect
                ) {
                    tasks.push(self.save_cache_in_background(true));
                }
                tasks.push(self.hashtags.update(message));
            }
            Message::Lists(message) => {
                if matches!(
                    &message,
                    lists::Message::Select(_) | lists::Message::Deselect
                ) {
                    tasks.push(self.save_cache_in_background(true));
                }
                tasks.push(self.lists.update(message));
            }
            Message::Search(message) => {
                if matches!(
                    &message,
                    search::Message::SelectTag(_)
                        | search::Message::DeselectTag
                        | search::Message::Clear
                ) {
                    tasks.push(self.save_cache_in_background(true));
                }
                tasks.push(self.search.update(message));
            }
            Message::Social(message) => {
                tasks.push(self.social.update(message, self.mastodon.clone()))
            }
            Message::Settings(message) => match message {
                settings::Message::ToggleDesktopNotifications(enabled) => {
                    self.config.disable_desktop_notifications = !enabled;
                    if let Some(handler) = &self.handler {
                        let _ = self
                            .config
                            .set_disable_desktop_notifications(handler, !enabled);
                    }
                }
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
                settings::Message::VerifySession => {
                    self.session_expired = false;
                    tasks.push(fetch_session_info(self.mastodon.clone()));
                }
                settings::Message::ConfirmClearCache => self.confirm_clear_cache = true,
                settings::Message::CancelClearCache if !self.clearing_cache => {
                    self.confirm_clear_cache = false
                }
                settings::Message::ClearCache
                    if self.confirm_clear_cache && !self.clearing_cache =>
                {
                    self.clearing_cache = true;
                    let account = self.mastodon.cache_key();
                    tasks.push(cosmic::task::future(async move {
                        let result = tokio::task::spawn_blocking(move || {
                            crate::persistence::clear_cached_content(&account)
                        })
                        .await;
                        Message::CacheCleared(match result {
                            Ok(Ok(())) => Ok(()),
                            _ => Err(fl!("settings-cache-error")),
                        })
                    }));
                }
                _ => (),
            },
            Message::Account(message) => match message {
                accounts::Message::People(id, following) => {
                    tasks.push(self.update(Message::ToggleContextPage(ContextPage::Social(
                        crate::features::social::Page::People(id, following),
                    ))))
                }
                accounts::Message::EditProfile => {
                    tasks.push(self.update(Message::ToggleContextPage(ContextPage::Social(
                        crate::features::social::Page::EditProfile,
                    ))))
                }
                accounts::Message::Private(account) => {
                    tasks.push(self.update(Message::Compose(compose::Message::Private(account))))
                }
                accounts::Message::Follow(id, following) => {
                    let mastodon = self.mastodon.clone();
                    tasks.push(cosmic::task::future(async move {
                        let result = crate::client::within_timeout(
                            async {
                                if following {
                                    mastodon.unfollow_account(id).await
                                } else {
                                    mastodon.follow_account(id, None).await
                                }
                            },
                            crate::fl!("action-error"),
                        )
                        .await;
                        match result {
                            Ok(response) => Message::CacheRelationship(response.json),
                            Err(err) => Message::Error(err),
                        }
                    }))
                }
                accounts::Message::Mute(id, muting) => {
                    let mastodon = self.mastodon.clone();
                    tasks.push(cosmic::task::future(async move {
                        let result = crate::client::within_timeout(
                            async {
                                if muting {
                                    mastodon.unmute_account(id).await
                                } else {
                                    mastodon.mute_account(id, true).await
                                }
                            },
                            crate::fl!("action-error"),
                        )
                        .await;
                        match result {
                            Ok(response) => Message::CacheRelationship(response.json),
                            Err(err) => Message::Error(err),
                        }
                    }))
                }
                accounts::Message::Block(id, blocking) => {
                    let mastodon = self.mastodon.clone();
                    tasks.push(cosmic::task::future(async move {
                        let result = crate::client::within_timeout(
                            async {
                                if blocking {
                                    mastodon.unblock_account(id).await
                                } else {
                                    mastodon.block_account(id).await
                                }
                            },
                            crate::fl!("action-error"),
                        )
                        .await;
                        match result {
                            Ok(response) => Message::CacheRelationship(response.json),
                            Err(err) => Message::Error(err),
                        }
                    }))
                }
                _ => tasks.push(accounts::update(message)),
            },
            Message::Status(message) => match message {
                status::Message::Menu(id) => self.cache.reading.post_menu = id,
                status::Message::MenuAction(message) => {
                    self.cache.reading.post_menu = None;
                    return self.update(Message::Status(*message));
                }
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
                    let previous = self.cache.statuses.get(&status_id);
                    let favorited = previous
                        .and_then(|post| post.favourited)
                        .unwrap_or(favorited);
                    let count = previous.map(|post| post.favourites_count);
                    tasks.push(cosmic::task::future(async move {
                        let result =
                            status::favorite::set(&mastodon, status_id, favorited, count).await;
                        match result {
                            Ok(status) => Message::FavoriteChanged(status),
                            Err(err) => Message::Error(err),
                        }
                    }))
                }
                status::Message::Boost(status_id, boosted) => {
                    let mastodon = self.mastodon.clone();
                    let previous = self.cache.statuses.get(&status_id);
                    let boosted = previous.and_then(|post| post.reblogged).unwrap_or(boosted);
                    let count = previous.map(|post| post.reblogs_count);
                    tasks.push(cosmic::task::future(async move {
                        let result = status::boost::set(&mastodon, status_id, boosted, count).await;
                        match result {
                            Ok(status) => Message::CacheStatus(status),
                            Err(err) => Message::Error(err),
                        }
                    }))
                }
                status::Message::Bookmark(status_id, bookmarked) => {
                    let mastodon = self.mastodon.clone();
                    tasks.push(cosmic::task::future(async move {
                        let result = crate::client::within_timeout(
                            async {
                                if bookmarked {
                                    mastodon.unbookmark_status(status_id).await
                                } else {
                                    mastodon.bookmark_status(status_id).await
                                }
                            },
                            crate::fl!("action-error"),
                        )
                        .await;
                        match result {
                            Ok(response) => Message::BookmarkChanged(response.json),
                            Err(err) => Message::Error(err),
                        }
                    }))
                }
                _ => tasks.push(status::update(message)),
            },
            Message::CacheHandle(epoch, url, handle) => {
                if epoch != crate::persistence::image_epoch() {
                    return Task::none();
                }
                self.image_inflight.remove(&url);
                self.cache.insert_handle(url, handle);
                tasks.push(self.drain_image_queue());
            }
            Message::ImageFetchFailed(epoch, url) => {
                if epoch != crate::persistence::image_epoch() {
                    return Task::none();
                }
                self.image_inflight.remove(&url);
                tasks.push(self.drain_image_queue());
            }
            Message::FavoriteChanged(status) => {
                self.cache.insert_status(status.clone());
                tasks.push(
                    self.favorites
                        .update(timeline::Message::CollectionChanged(status)),
                );
            }
            Message::BookmarkChanged(status) => {
                self.cache.insert_status(status.clone());
                tasks.push(
                    self.bookmarks
                        .update(timeline::Message::CollectionChanged(status)),
                );
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
                tasks.push(self.reset_and_reload_feeds());
            }
            Message::SetAccount(account) => {
                self.session_expired = false;
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
                if let Some(profile) = &mut self.profile {
                    if profile.account.id == account.id {
                        profile.account = account.clone();
                    }
                }
                self.cache.me = Some(account);
            }
            Message::SetComposeLimits(limits) => {
                self.compose_limits = limits;
            }
            Message::Compose(message) => tasks.push(self.update_compose(message)),
            Message::StatusDeleted(id) => {
                let deleted = self.cache.delete_status(&id);
                for id in deleted {
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
                    tasks.push(self.hashtags.update(hashtags::Message::Timeline(
                        timeline::Message::DeleteStatus(id.clone()),
                    )));
                    tasks.push(self.lists.update(lists::Message::Timeline(
                        timeline::Message::DeleteStatus(id.clone()),
                    )));
                    tasks.push(self.search.update(search::Message::Timeline(
                        timeline::Message::DeleteStatus(id),
                    )));
                }
            }
            Message::Open(url) => {
                if !reading::html::web_url(&url) {
                    return Task::none();
                }
                tasks.push(self.open_browser(url, false));
            }
            Message::ToggleContextPage(context_page) => {
                if matches!(
                    context_page,
                    ContextPage::About
                        | ContextPage::Settings
                        | ContextPage::Social(crate::features::social::Page::People(_, _))
                ) {
                    if self.context_page == context_page {
                        self.core.window.show_context = !self.core.window.show_context;
                    } else {
                        self.context_page = context_page;
                        self.core.window.show_context = true;
                    }
                    if self.core.window.show_context {
                        if let ContextPage::Social(page) = &self.context_page {
                            tasks.push(self.social.open(page.clone(), self.mastodon.clone()));
                        }
                    }
                } else if self.main_page.as_ref() != Some(&context_page) {
                    if let ContextPage::Social(page) = &context_page {
                        tasks.push(self.social.open(page.clone(), self.mastodon.clone()));
                    }
                    if let ContextPage::Account(account) = &context_page {
                        if self.mastodon.is_authenticated() {
                            let mastodon = self.mastodon.clone();
                            let id = account.id.clone();
                            tasks.push(cosmic::task::future(async move {
                                match crate::client::within_timeout(
                                    mastodon.get_relationships(vec![id]),
                                    crate::fl!("action-error"),
                                )
                                .await
                                {
                                    Ok(response) => response
                                        .json
                                        .into_iter()
                                        .next()
                                        .map(Message::CacheRelationship)
                                        .unwrap_or(Message::None),
                                    Err(err) => Message::Error(err),
                                }
                            }));
                        }
                    }
                    if let Some(previous) = self.main_page.replace(context_page) {
                        self.context_history.push(previous);
                        if self.context_history.len() > 64 {
                            self.context_history.remove(0);
                        }
                    }
                    self.core.window.show_context = false;
                    tasks.push(self.load_reader());
                    tasks.push(self.update_title());
                } else {
                    self.core.window.show_context = false;
                }
            }
            Message::ToggleContextDrawer => {
                self.core.window.show_context = !self.core.window.show_context;
            }
            Message::VideoTick => {
                if let Some(video) = &mut self.video {
                    if let Some(frame) = video.tick() {
                        let id = frame.id();
                        tasks.push(
                            cosmic::iced::runtime::image::allocate(frame)
                                .map(move |result| Message::VideoFrameUploaded(id, result))
                                .map(cosmic::Action::App),
                        );
                    }
                }
            }
            Message::VideoFrameUploaded(id, result) => {
                if let Some(video) = &mut self.video {
                    video.frame_uploaded(id, result);
                }
            }
            Message::VideoPause => {
                if let Some(video) = &mut self.video {
                    video.toggle_pause();
                }
            }
            Message::VideoMute => {
                if let Some(video) = &mut self.video {
                    video.toggle_mute();
                }
            }
            Message::VideoSeek(position) => {
                if let Some(video) = &mut self.video {
                    video.scrub(position);
                }
            }
            Message::VideoSeekReleased => {
                if let Some(video) = &mut self.video {
                    video.finish_scrub();
                }
            }
            Message::ToggleMediaDetails => self.media_details = !self.media_details,
            Message::RetryMediaThread => tasks.push(self.load_media_thread()),
            Message::MediaStatus(message) => {
                let message = match *message {
                    status::Message::MenuAction(message) => {
                        self.cache.reading.post_menu = None;
                        message
                    }
                    message => Box::new(message),
                };
                if matches!(
                    *message,
                    status::Message::OpenAccount(_)
                        | status::Message::ExpandStatus(_)
                        | status::Message::Reply(_)
                        | status::Message::Edit(_)
                        | status::Message::Report(..)
                        | status::Message::Delete(_)
                        | status::Message::InlineLink(..)
                        | status::Message::ViewMedia(..)
                ) {
                    self.dialog_pages.pop_front();
                    self.media_thread = None;
                }
                tasks.push(self.update(Message::Status(*message)));
            }
            Message::NavigateMedia(forward) => {
                if let Some(Dialog::Media(images, index, _)) = self.dialog_pages.front_mut() {
                    if forward {
                        if *index + 1 < images.len() {
                            *index += 1;
                        }
                    } else {
                        *index = index.saturating_sub(1);
                    }
                }
            }
            Message::Dialog(action) => match action {
                DialogAction::Open(dialog) => match dialog {
                    Dialog::Compose(state) => return self.open_compose(state),
                    Dialog::Media(items, index, id) => {
                        if index < items.len() {
                            self.dialog_pages.push_back(Dialog::Media(items, index, id));
                            tasks.push(self.load_media_thread());
                        }
                    }
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
                        if matches!(self.dialog_pages.front(), Some(Dialog::Media(..))) {
                            self.media_thread = None;
                            self.cache.reading.post_menu = None;
                        }
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
                            Dialog::Drafts
                            | Dialog::EditLoading { .. }
                            | Dialog::CloseDraft
                            | Dialog::AltText(_) => {}
                            Dialog::DeleteStatus(id) => {
                                let mastodon = self.mastodon.clone();
                                tasks.push(cosmic::task::future(async move {
                                    match crate::client::within_timeout(
                                        mastodon.delete_status(id.clone()),
                                        crate::fl!("action-error"),
                                    )
                                    .await
                                    {
                                        Ok(_) => Message::StatusDeleted(id),
                                        Err(err) => Message::Error(err),
                                    }
                                }));
                            }
                            Dialog::SwitchInstance(instance) => {
                                let Ok(server) = onboarding::normalize_server(&instance) else {
                                    return Task::none();
                                };
                                self.instance = server;
                                tasks.push(self.update(Message::InstanceEdit));
                                tasks.push(self.update(Message::UpdateMastodonInstance))
                            }
                            Dialog::Logout => {
                                tasks.push(self.remove_account(self.sessions.active));
                            }
                            Dialog::Media(..) => {}
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
                    self.completion_serial += 1;
                    if let Some(state) = self.active_compose_mut() {
                        state.suggestions.clear();
                    }
                    if self.dialog_editor.cursor().selection.is_none() {
                        let cursor = self.dialog_editor.cursor().position;
                        if let Some((_, query)) = compose::completion::token(
                            &self.dialog_editor.text(),
                            cursor.line,
                            cursor.column,
                        ) {
                            let serial = self.completion_serial;
                            tasks.push(cosmic::task::future(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                                Message::Compose(compose::Message::QuerySuggestions(serial, query))
                            }));
                        }
                    }
                    if editing {
                        let text = self.dialog_editor.text();
                        if let Some(state) = self.active_compose_mut() {
                            state.text = text;
                            state.changed();
                        }
                        self.stage_active_draft();
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
            Message::FlushCache => tasks.push(self.save_cache_in_background(false)),
            Message::SaveDraft => {
                if !self.draft_saving
                    && self
                        .draft_save_due
                        .is_some_and(|due| due <= std::time::Instant::now())
                {
                    self.draft_saving = true;
                    let revision = self.draft_revision;
                    let store = self.compose_store.clone();
                    let ticket = store.reserve_save();
                    let account = self.mastodon.cache_key();
                    let drafts = self.drafts.clone();
                    tasks.push(cosmic::task::future(async move {
                        let result = tokio::task::spawn_blocking(move || {
                            store.save_reserved(&account, &drafts, ticket)
                        })
                        .await
                        .unwrap_or_else(|_| Err(fl!("compose-storage-error")));
                        Message::DraftSaved(revision, result)
                    }));
                }
            }
            Message::DraftSaved(revision, result) => {
                self.draft_saving = false;
                if revision == self.draft_revision {
                    self.draft_save_due = result
                        .as_ref()
                        .err()
                        .map(|_| std::time::Instant::now() + std::time::Duration::from_secs(5));
                    if let Some(state) = self.active_compose_mut() {
                        state.storage_error = result.err();
                    }
                }
            }
            Message::CacheSaved(revision, result) => {
                if self.cache_saving == Some(revision) {
                    self.cache_saving = None;
                }
                if let Err(error) = result {
                    self.cache.dirty = true;
                    tracing::warn!("failed to save feed snapshots: {error}");
                }
            }
            Message::MyProfile => {
                if let Some(account) = self.cache.me.clone() {
                    tasks.push(
                        self.update(Message::ToggleContextPage(ContextPage::Account(account))),
                    );
                }
            }
            Message::None => (),
        }
        self.sync_video();
        let image_url = match self.dialog_pages.front() {
            Some(Dialog::Media(items, index, _)) => items
                .get(*index)
                .filter(|item| !item.video)
                .map(|item| item.url.clone()),
            _ => None,
        };
        if self.gallery_image_url != image_url {
            self.gallery_image_url = image_url.clone();
            if let Some(url) = image_url {
                tasks.push(cosmic::task::message(Message::Fetch(vec![url])));
            }
        }
        scoped_task(Task::batch(tasks), self.generation)
    }
}

impl AppModel {
    fn open_browser(&self, url: String, onboarding: bool) -> Task<Message> {
        #[cfg(target_os = "linux")]
        let token = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            cosmic::iced::platform_specific::shell::wayland::commands::activation::request_token(
                Some(Self::APP_ID.to_owned()),
                self.core.main_window_id(),
            )
        } else {
            cosmic::iced::Task::done(None)
        };
        #[cfg(not(target_os = "linux"))]
        let token = cosmic::iced::Task::done(None::<String>);

        token.then(move |token| {
            let url = url.clone();
            cosmic::task::future(async move {
                let result =
                    tokio::task::spawn_blocking(move || launch_browser(&url, token.as_deref()))
                        .await;
                match result {
                    Ok(Ok(())) => Message::None,
                    _ if onboarding => Message::Onboarding(onboarding::Message::BrowserFailed),
                    _ => Message::Error(fl!("action-error")),
                }
            })
        })
    }

    fn sync_video(&mut self) {
        let attachment = match self.dialog_pages.front() {
            Some(Dialog::Media(items, index, _)) => items.get(*index).filter(|item| item.video),
            _ => None,
        };
        let url = attachment.map(|item| item.url.clone());
        if self.video_url != url {
            self.video = None;
            self.video_failed = false;
            if let Some(item) = attachment {
                match crate::media::Player::new(&item.url, item.looping) {
                    Ok(player) => self.video = Some(player),
                    Err(()) => self.video_failed = true,
                }
            }
            self.video_url = url;
        }
    }
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
    fn stage_active_draft(&mut self) -> bool {
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
        self.draft_revision += 1;
        self.draft_save_due =
            Some(std::time::Instant::now() + std::time::Duration::from_millis(400));
        true
    }

    fn persist_active_draft(&mut self) -> bool {
        if !self.stage_active_draft() {
            return false;
        }
        if self.active_compose().is_none() {
            return true;
        }
        let key = self.mastodon.cache_key();
        let result = self.compose_store.save(&key, &self.drafts);
        if let Some(state) = self.active_compose_mut() {
            state.storage_error = result.as_ref().err().cloned();
        }
        if result.is_ok() {
            self.draft_save_due = None;
        }
        result.is_ok()
    }
    fn open_compose(&mut self, state: compose::State) -> Task<Message> {
        if self.active_compose().is_some()
            || self
                .dialog_pages
                .iter()
                .any(|dialog| matches!(dialog, Dialog::EditLoading { .. }))
        {
            return cosmic::task::message(Message::Error(fl!("compose-existing")));
        }
        if self.cache.me.is_none() || self.mastodon.account_id.is_empty() {
            return cosmic::task::message(Message::Error(fl!("compose-account-loading")));
        }
        self.completion_serial += 1;
        self.compose_request += 1;
        self.dialog_editor = widget::text_editor::Content::with_text(&state.text);
        self.dialog_pages
            .retain(|dialog| !matches!(dialog, Dialog::Drafts));
        let previews = state.preview_urls();
        let uploads: Vec<_> = state
            .media
            .iter()
            .filter(|media| state.accepts_edits() && media.needs_upload())
            .map(|media| media.id.clone())
            .collect();
        self.dialog_pages.push_front(Dialog::Compose(state));
        self.persist_active_draft();
        Task::batch(
            std::iter::once(cosmic::task::message(Message::Fetch(previews))).chain(
                uploads.into_iter().map(|id| {
                    cosmic::task::message(Message::Compose(compose::Message::Upload(id)))
                }),
            ),
        )
    }
    fn close_compose_frames(&mut self) {
        self.completion_serial += 1;
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
        self.draft_revision += 1;
        self.draft_save_due = None;
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
            C::QuerySuggestions(serial, query)
                if serial == self.completion_serial
                    && self.active_compose().is_some_and(compose::State::editable) =>
            {
                let client = self.mastodon.clone();
                return cosmic::task::future(async move {
                    Message::Compose(C::Suggestions(
                        serial,
                        compose::completion::fetch(client, query).await,
                    ))
                });
            }
            C::Suggestions(serial, suggestions) if serial == self.completion_serial => {
                if let Some(state) = self.active_compose_mut().filter(|state| state.editable()) {
                    state.suggestions = suggestions;
                }
            }
            C::Complete(value)
                if self.active_compose().is_some_and(|state| {
                    state.editable()
                        && state
                            .suggestions
                            .iter()
                            .any(|suggestion| suggestion.value == value)
                }) =>
            {
                use unicode_segmentation::UnicodeSegmentation;
                let cursor = self.dialog_editor.cursor();
                if cursor.selection.is_none() {
                    let mut text = self.dialog_editor.text();
                    if let Some((range, query)) = compose::completion::token(
                        &text,
                        cursor.position.line,
                        cursor.position.column,
                    ) {
                        let inserted = format!("{value} ");
                        text.replace_range(range, &inserted);
                        self.dialog_editor = widget::text_editor::Content::with_text(&text);
                        let mut cursor = cursor;
                        cursor.position.column = cursor
                            .position
                            .column
                            .saturating_sub(query.graphemes(true).count())
                            + inserted.graphemes(true).count();
                        self.dialog_editor.move_to(cursor);
                        self.completion_serial += 1;
                        if let Some(state) = self.active_compose_mut() {
                            state.text = text;
                            state.suggestions.clear();
                            state.changed();
                        }
                        self.persist_active_draft();
                    }
                }
            }
            C::QuerySuggestions(..) | C::Suggestions(..) | C::Complete(..) => (),
            C::Private(account) => {
                let mut state = compose::State::new(self.cache.me.as_ref());
                state.visibility = megalodon::entities::status::StatusVisibility::Direct;
                state.reply_ceiling = Some(state.visibility.clone());
                state.text = format!("{} ", reading::html::full_acct(&account));
                return self.open_compose(state);
            }
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
                if !self.mastodon.is_authenticated() {
                    return cosmic::task::message(Message::Error(fl!("compose-account-loading")));
                }
                self.completion_serial += 1;
                self.compose_request += 1;
                let request = self.compose_request;
                self.dialog_pages
                    .retain(|dialog| !matches!(dialog, Dialog::EditLoading { .. }));
                self.dialog_pages.push_front(Dialog::EditLoading {
                    id: id.clone(),
                    request,
                    error: None,
                });
                let client = self.mastodon.clone();
                return cosmic::task::future(async move {
                    Message::Compose(C::EditLoaded(
                        request,
                        compose::network::edit(client, id).await,
                    ))
                });
            }
            C::EditLoaded(request, result) => {
                if request != self.compose_request {
                    return Task::none();
                }
                let Some(index) = self.dialog_pages.iter().position(|dialog|
                    matches!(dialog, Dialog::EditLoading { request: pending, .. } if *pending == request))
                else { return Task::none(); };
                return match result {
                    Ok(state) => {
                        self.dialog_pages.remove(index);
                        self.open_compose(state)
                    }
                    Err(error) => {
                        if let Dialog::EditLoading { error: current, .. } =
                            &mut self.dialog_pages[index]
                        {
                            *current = Some(error);
                        }
                        Task::none()
                    }
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
                            && !state.has_poll
                            && state.poll_options.is_empty() =>
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
                            self.stage_active_draft();
                            return Task::batch(
                                ids.into_iter().map(|id| {
                                    cosmic::task::message(Message::Compose(C::Upload(id)))
                                }),
                            );
                        }
                        Err(error) => state.error = Some(error),
                    }
                }
                self.stage_active_draft();
            }
            C::Upload(id) => {
                let Some(state) = self.active_compose().filter(|state| state.accepts_edits())
                else {
                    return Task::none();
                };
                let Some(media) = state
                    .media
                    .iter()
                    .find(|media| media.id == id && media.needs_upload())
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
                self.stage_active_draft();
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
                            state.published_scheduled = published.scheduled;
                        }
                        if self.persist_active_draft() {
                            self.drop_active_draft();
                            self.close_compose_frames();
                        }
                        let mut tasks = vec![self
                            .toasts
                            .push(Toast::new(if editing {
                                fl!("compose-edited")
                            } else if published.scheduled {
                                fl!("compose-scheduled")
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
                            for thread in self.thread.iter_mut().chain(self.media_thread.iter_mut())
                            {
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
                self.stage_active_draft();
            }
            C::Tick => (),
            change => {
                if let Some(state) = self.active_compose_mut().filter(|state| state.editable()) {
                    match change {
                        C::TogglePoll(enabled)
                            if state.edit_id.is_none() && state.media.is_empty() =>
                        {
                            state.poll_options = if enabled {
                                vec![String::new(), String::new()]
                            } else {
                                vec![]
                            }
                        }
                        C::PollOption(index, value) => {
                            if let Some(option) = state.poll_options.get_mut(index) {
                                *option = value;
                            }
                        }
                        C::AddPollOption => state.poll_options.push(String::new()),
                        C::RemovePollOption(index)
                            if state.poll_options.len() > 2 && index < state.poll_options.len() =>
                        {
                            state.poll_options.remove(index);
                        }
                        C::PollMultiple(value) => state.poll_multiple = value,
                        C::PollDuration(value) => state.poll_duration = value,
                        C::Schedule(value) if state.edit_id.is_none() => state.schedule = value,
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
                self.stage_active_draft();
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
            Effect::OpenBrowser(url) => self.open_browser(url, true),
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
        self.draft_saving = false;
        self.draft_save_due = None;
        self.cache_saving = None;
        self.video = None;
        self.video_url = None;
        self.social = crate::features::social::Social::default();
        self.activity = notifications::desktop::Activity::load(&self.mastodon.cache_key());
        self.update_activity_badge();
        self.generation += 1;
        self.filters_loading = false;
        self.stream_state = crate::streaming::State::Connecting;
        self.session_expired = false;
        self.confirm_clear_cache = false;
        self.clearing_cache = false;
        self.filter_request += 1;
        self.thread = None;
        self.media_thread = None;
        self.profile = None;
        self.main_page = None;
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
        if self.clearing_cache {
            return Task::none();
        }
        let mut tasks = vec![];
        while self.image_inflight.len() < MAX_CONCURRENT_IMAGE_FETCHES {
            let Some(url) = self.image_queue.pop_front() else {
                break;
            };
            self.image_inflight.insert(url.clone());
            let epoch = crate::persistence::image_epoch();
            tasks.push(cosmic::task::future(async move {
                let result = crate::cache::get(&url, epoch).await;
                if epoch != crate::persistence::image_epoch() {
                    return Message::ImageFetchFailed(epoch, url);
                }
                match result {
                    Ok(handle) => Message::CacheHandle(epoch, url, handle),
                    Err(err) => {
                        tracing::error!("Failed to fetch image: {}", err);
                        Message::ImageFetchFailed(epoch, url)
                    }
                }
            }));
        }
        Task::batch(tasks)
    }

    fn prune_cache(&mut self) {
        let mut statuses: HashSet<String> = [
            &self.home,
            &self.explore,
            &self.local,
            &self.federated,
            &self.favorites,
            &self.bookmarks,
        ]
        .into_iter()
        .chain(self.hashtags.timeline())
        .chain(self.lists.timeline())
        .chain(self.search.timeline())
        .flat_map(Timeline::status_ids)
        .cloned()
        .collect();
        let mut images = HashSet::new();
        let mut accounts = HashSet::new();
        let mut keep_account = |account: &Account| {
            accounts.insert(account.id.clone());
            images.extend([account.avatar.clone(), account.header.clone()]);
        };
        if let Some(account) = &self.cache.me {
            keep_account(account);
        }
        for account in self.social.accounts() {
            keep_account(account);
        }
        if let Some(results) = self.search.results() {
            statuses.extend(results.statuses.iter().map(|post| post.id.clone()));
            for account in &results.accounts {
                keep_account(account);
            }
        }
        statuses.extend(self.social.statuses().map(|post| post.id.clone()));
        for thread in self.thread.iter().chain(self.media_thread.iter()) {
            statuses.insert(thread.root.clone());
            statuses.extend(thread.ancestors.iter().chain(&thread.descendants).cloned());
        }
        if let Some(profile) = &self.profile {
            keep_account(&profile.account);
            statuses.extend(profile.pinned.iter().chain(&profile.posts).cloned());
        }
        for page in self
            .context_history
            .iter()
            .chain(self.main_page.iter())
            .chain(std::iter::once(&self.context_page))
        {
            match page {
                ContextPage::Status(id) => {
                    statuses.insert(id.clone());
                }
                ContextPage::Account(account) => keep_account(account),
                _ => (),
            }
        }
        if let Some(state) = self.active_compose() {
            statuses.extend(state.in_reply_to_id.iter().cloned());
            images.extend(state.preview_urls());
        }
        if let Some(Dialog::Media(items, index, _)) = self.dialog_pages.front() {
            images.extend(items.iter().filter_map(|item| item.preview.clone()));
            if let Some(item) = items.get(*index) {
                images.insert(item.url.clone());
            }
        }
        let notifications = self.notifications.notification_ids().cloned().collect();
        let images = self
            .cache
            .retain_referenced(statuses, &notifications, images, accounts);
        self.image_queue.retain(|url| images.contains(url));
        for url in images {
            if !url.is_empty()
                && !self.cache.handles.contains_key(&url)
                && !self.image_inflight.contains(&url)
                && !self.image_queue.contains(&url)
            {
                self.image_queue.push_back(url);
            }
        }
    }

    fn cache_snapshots(&self) -> Vec<crate::persistence::Snapshot> {
        [
            self.home.snapshot(&self.cache),
            self.explore.snapshot(&self.cache),
            self.local.snapshot(&self.cache),
            self.federated.snapshot(&self.cache),
            self.favorites.snapshot(&self.cache),
            self.bookmarks.snapshot(&self.cache),
            self.notifications.snapshot(&self.cache),
        ]
        .into_iter()
        .flatten()
        .chain(
            [
                self.hashtags.timeline(),
                self.lists.timeline(),
                self.search.timeline(),
            ]
            .into_iter()
            .flatten()
            .filter_map(|timeline| timeline.snapshot(&self.cache)),
        )
        .collect()
    }

    fn save_cache_in_background(&mut self, leaving_scope: bool) -> Task<Message> {
        self.prune_cache();
        let images = self.drain_image_queue();
        if self.clearing_cache
            || (!leaving_scope && (!self.cache.dirty || self.cache_saving.is_some()))
        {
            return images;
        }
        let snapshots = self.cache_snapshots();
        let account = self.mastodon.cache_key();
        let revision = crate::persistence::reserve_snapshot_save();
        self.cache.dirty = false;
        self.cache_saving = Some(revision);
        Task::batch([
            images,
            cosmic::task::future(async move {
                let result = tokio::task::spawn_blocking(move || {
                    crate::persistence::save_snapshots(&account, revision, snapshots)
                })
                .await
                .unwrap_or_else(|error| Err(error.to_string()));
                Message::CacheSaved(revision, result)
            }),
        ])
    }

    // Account changes and exit need a completed snapshot before proceeding.
    fn flush_cache_to_disk(&mut self) {
        if (!self.cache.dirty && self.cache_saving.is_none()) || self.clearing_cache {
            return;
        }
        let snapshots = self.cache_snapshots();
        let revision = crate::persistence::reserve_snapshot_save();
        match crate::persistence::save_snapshots(&self.mastodon.cache_key(), revision, snapshots) {
            Ok(()) => self.cache.dirty = false,
            Err(error) => tracing::warn!("failed to save feed snapshots: {error}"),
        }
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
        let mut tasks = vec![self.activate_session(Some(session))];
        if let Err(err) = self.persist_sessions() {
            tasks.push(cosmic::task::message(Message::Error(format!(
                "Couldn't save session: {err}"
            ))));
        }
        Task::batch(tasks)
    }

    fn activate_session(&mut self, session: Option<Session>) -> Task<Message> {
        let onboarding_completed = self.config.onboarding_completed;
        self.config = session
            .as_ref()
            .and_then(|session| session.preferences.clone())
            .unwrap_or_else(|| TootConfig {
                server: self.config.server.clone(),
                ..Default::default()
            });
        self.config.onboarding_completed = onboarding_completed;
        self.mastodon = match session {
            Some(session) => {
                self.instance = session.base_url.clone();
                Client::from_session(&session)
            }
            None => Client::new(self.instance(), None),
        };
        self.cache.clear();
        self.cache.hide_boosts = self.config.hide_boosts;
        self.cache.hide_replies = self.config.hide_replies;
        self.cache.feed_density = self.config.feed_density;
        self.update_navbar();

        let mut tasks = vec![
            self.reset_and_reload_feeds(),
            self.on_nav_select(self.nav.active()),
            cosmic::command::set_theme(self.config.theme_mode.theme()),
        ];
        if self.mastodon.is_authenticated() {
            tasks.push(fetch_session_info(self.mastodon.clone()));
        }
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
            notifications::desktop::clear(&base_url);
        }

        let mut tasks = vec![];
        if let Err(err) = self.persist_sessions() {
            tasks.push(cosmic::task::message(Message::Error(format!(
                "Couldn't save session: {err}"
            ))));
        }

        if was_active {
            tasks.push(self.activate_session(self.sessions.active_session().cloned()));
        }
        Task::batch(tasks)
    }

    pub fn update_title(&mut self) -> Task<Message> {
        let mut window_title = fl!("app-title");
        if let Some(page) = &self.main_page {
            window_title.push_str(" — ");
            window_title.push_str(&page.title());
        } else if let Some(page) = self.nav.text(self.nav.active()) {
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
        let valid = onboarding::normalize_server(&instance).is_ok();
        widget::dialog()
            .title(fl!("server-question"))
            .body(fl!("guest-server-description"))
            .icon(widget::icon::from_name("network-server-symbolic"))
            .control(
                widget::text_input(fl!("server-url"), instance)
                    .on_input(|instance| {
                        Message::Dialog(DialogAction::Update(Dialog::SwitchInstance(instance)))
                    })
                    .on_submit(move |_| {
                        if valid {
                            Message::Dialog(DialogAction::Complete)
                        } else {
                            Message::None
                        }
                    }),
            )
            .primary_action(
                widget::button::suggested(fl!("confirm"))
                    .on_press_maybe(valid.then_some(Message::Dialog(DialogAction::Complete))),
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

    /// A full-window media overlay, in the same spirit as cosmic-files'
    /// gallery view: a translucent backdrop covering the whole window with
    /// the image centered and content-fit, rather than a small dialog box.
    fn view_media(
        &self,
        images: &[crate::media::Attachment],
        index: usize,
    ) -> Element<'_, Message> {
        let spacing = cosmic::theme::active().cosmic().spacing;

        let attachment = &images[index];
        let original_url = &attachment.url;
        let failed = self.video_failed || self.video.as_ref().is_some_and(|video| video.error);
        let handle = self
            .video
            .as_ref()
            .and_then(crate::media::Player::frame)
            .or_else(|| attachment.cached_image(&self.cache.handles));
        let image: Element<'_, Message> = if attachment.video && failed {
            widget::text(fl!("media-playback-error")).into()
        } else if let Some(handle) = handle {
            widget::image(handle)
                .content_fit(cosmic::iced::ContentFit::Contain)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            widget::text(if attachment.video {
                fl!("media-loading")
            } else {
                fl!("media-image-unavailable")
            })
            .into()
        };

        // Matching bars keep the media centered and leave room for playback controls.
        let bar_height = Length::Fixed(96.0);

        let header_row = widget::row![
            widget::space::horizontal(),
            crate::ui::icon_action(
                "view-dual-symbolic",
                if self.media_details {
                    fl!("media-hide-details")
                } else {
                    fl!("media-show-details")
                },
                Some(Message::ToggleMediaDetails)
            ),
            crate::ui::icon_action(
                "send-to-symbolic",
                fl!("media-open"),
                Some(Message::Open(original_url.clone()))
            ),
            crate::ui::icon_action(
                "window-close-symbolic",
                fl!("media-close"),
                Some(Message::Dialog(DialogAction::Close))
            ),
        ]
        .align_y(cosmic::iced::Alignment::Center)
        .spacing(spacing.space_xs)
        .padding(spacing.space_xs);

        let header = widget::container(header_row)
            .width(Length::Fill)
            .height(bar_height)
            .align_y(Vertical::Center);

        let mut controls = widget::row![]
            .spacing(spacing.space_s)
            .align_y(cosmic::iced::Alignment::Center);
        if let Some(video) = &self.video {
            controls = controls
                .push(crate::ui::icon_action(
                    if video.paused {
                        "media-playback-start-symbolic"
                    } else {
                        "media-playback-pause-symbolic"
                    },
                    if video.paused {
                        fl!("media-play")
                    } else {
                        fl!("media-pause")
                    },
                    (!video.error).then_some(Message::VideoPause),
                ))
                .push(crate::ui::icon_action(
                    if video.muted {
                        "audio-volume-muted-symbolic"
                    } else {
                        "audio-volume-high-symbolic"
                    },
                    if video.muted {
                        fl!("media-unmute")
                    } else {
                        fl!("media-mute")
                    },
                    (!video.error).then_some(Message::VideoMute),
                ));
            if video.duration > 0.0 && !video.error {
                controls = controls
                    .push(
                        widget::slider(
                            0.0..=video.duration as f32,
                            video.display_position().min(video.duration) as f32,
                            |value| Message::VideoSeek(value as f64),
                        )
                        .on_release(Message::VideoSeekReleased)
                        .width(180),
                    )
                    .push(widget::text(format!(
                        "{}:{:02} / {}:{:02}",
                        video.display_position() as u64 / 60,
                        video.display_position() as u64 % 60,
                        video.duration as u64 / 60,
                        video.duration as u64 % 60
                    )));
            }
        }
        let footer = widget::container(
            widget::column![
                controls,
                widget::row![
                    crate::ui::icon_action(
                        "go-previous-symbolic",
                        fl!("media-previous"),
                        (index > 0).then_some(Message::NavigateMedia(false))
                    ),
                    widget::text(format!("{} / {}", index + 1, images.len())),
                    crate::ui::icon_action(
                        "go-next-symbolic",
                        fl!("media-next"),
                        (index + 1 < images.len()).then_some(Message::NavigateMedia(true))
                    ),
                ]
                .spacing(spacing.space_s)
                .align_y(cosmic::iced::Alignment::Center),
            ]
            .spacing(spacing.space_xs)
            .align_x(cosmic::iced::Alignment::Center),
        )
        .align_x(cosmic::iced::alignment::Horizontal::Center)
        .align_y(Vertical::Center)
        .width(Length::Fill)
        .height(bar_height);

        let mut content = widget::column![header, widget::container(image).center(Length::Fill),];
        let description = attachment
            .description
            .clone()
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| fl!("reading-no-description"));
        content = content.push(
            widget::container(
                widget::scrollable(
                    widget::selectable_text(description)
                        .width(Length::Fill)
                        .center(),
                )
                .height(80.0),
            )
            .padding([spacing.space_xs, spacing.space_m])
            .width(Length::Fill),
        );

        let media = content
            .push(footer)
            .width(Length::FillPortion(2))
            .height(Length::Fill);
        let mut layout = widget::row![media].height(Length::Fill);
        if self.media_details {
            if let Some(thread) = &self.media_thread {
                let details = thread.media_view(&self.cache);
                layout = layout.push(
                    widget::container(widget::scrollable(details).height(Length::Fill))
                        .padding(spacing.space_s)
                        .width(Length::FillPortion(1))
                        .max_width(420.)
                        .height(Length::Fill),
                );
            }
        }
        layout
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

    fn load_media_thread(&mut self) -> Task<Message> {
        let Some(Dialog::Media(_, _, id)) = self.dialog_pages.front() else {
            return Task::none();
        };
        let id = id.clone();
        self.reader_request += 1;
        let request = self.reader_request;
        self.media_thread = Some(reading::thread::Thread::new(request, id.clone()));
        let client = self.mastodon.clone();
        cosmic::task::future(async move {
            Message::ThreadLoaded(request, reading::thread::fetch(client, id).await)
        })
    }

    fn load_reader(&mut self) -> Task<Message> {
        self.reader_request += 1;
        let request = self.reader_request;
        self.thread = None;
        self.profile = None;
        match self.main_page.clone() {
            Some(ContextPage::Status(id)) => {
                self.thread = Some(reading::thread::Thread::new(request, id.clone()));
                let client = self.mastodon.clone();
                cosmic::task::future(async move {
                    Message::ThreadLoaded(request, reading::thread::fetch(client, id).await)
                })
            }
            Some(ContextPage::Account(account)) => {
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
        if let Some(page) = &self.main_page {
            view = view.push(
                widget::button::text(fl!("reading-back"))
                    .leading_icon(widget::icon::from_name("go-previous-symbolic"))
                    .on_press(Message::BackContext),
            );
            if !matches!(page, ContextPage::Account(_) | ContextPage::Status(_)) {
                view = view.push(widget::text::title3(page.title()));
            }
        }
        if matches!(
            self.main_page,
            Some(ContextPage::Account(_) | ContextPage::Status(_))
        ) {
            return widget::scrollable(
                widget::container(
                    widget::container(
                        view.push(content)
                            .spacing(crate::ui::GAP)
                            .width(Length::Fill),
                    )
                    .max_width(crate::ui::CONTENT_WIDTH)
                    .padding(16),
                )
                .center_x(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        }
        view.push(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
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

/// Pass activation only to the launch helpers, without changing the app's environment.
fn launch_browser(url: &str, token: Option<&str>) -> std::io::Result<()> {
    let mut last_error = std::io::Error::other("No browser launcher available");
    for mut command in open::commands(url) {
        if let Some(token) = token {
            command.env("XDG_ACTIVATION_TOKEN", token);
            command.env("DESKTOP_STARTUP_ID", token);
        }
        match command.status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => {
                last_error = std::io::Error::other(format!("Browser launcher exited with {status}"))
            }
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

/// Fetch the authenticated account and the instance's status length limit,
/// used to gate the compose dialog's delete action and character counter.
fn fetch_session_info(mastodon: Client) -> Task<Message> {
    let account_client = mastodon.clone();
    let instance_client = mastodon;
    Task::batch(vec![
        cosmic::task::future(async move {
            match tokio::time::timeout(
                std::time::Duration::from_secs(30),
                account_client.verify_account_credentials(),
            )
            .await
            {
                Ok(Ok(response)) => Message::SetAccount(response.json),
                Ok(Err(error)) if crate::client::unauthorized(&error) => Message::SessionExpired,
                _ => Message::Error(fl!("session-load-error")),
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
        // The guest default must allow unauthenticated public timelines.
        // mastodon.social requires a signed-in user for these endpoints.
        "https://mastodon.world".into()
    } else {
        format!("https://{}", instance)
    }
}

#[cfg(test)]
mod guest_tests {
    use super::instance;

    #[test]
    fn guest_default_preserves_explicit_server_choices() {
        assert_eq!(instance(""), "https://mastodon.world");
        assert_eq!(instance("  "), "https://mastodon.world");
        assert_eq!(instance("mastodon.social"), "https://mastodon.social");
        assert_eq!(instance(" https://example.org/ "), "https://example.org");
    }
}

impl AppModel
where
    Self: Application,
{
    fn instance(&self) -> String {
        instance(self.instance.clone())
    }

    fn update_activity_badge(&mut self) {
        let id = self
            .nav
            .iter()
            .find(|id| self.nav.data::<Page>(*id) == Some(&Page::Notifications));
        if let Some(id) = id {
            let count = self.activity.count();
            self.nav.text_set(
                id,
                if count == 0 {
                    fl!("notifications")
                } else {
                    format!("{} ({count})", fl!("notifications"))
                },
            );
        }
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
        self.update_activity_badge();
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum ContextPage {
    #[default]
    About,
    Account(Account),
    Status(String),
    Settings,
    Social(crate::features::social::Page),
}

impl ContextPage {
    fn title(&self) -> String {
        match self {
            ContextPage::About => fl!("about"),
            ContextPage::Account(_) => fl!("profile"),
            ContextPage::Status(_) => fl!("status"),
            ContextPage::Settings => fl!("feed-settings"),
            ContextPage::Social(page) => page.title(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    MyProfile,
    Conversations,
    Scheduled,
    Drafts,
    Logout,
    About,
    Settings,
}

impl menu::action::MenuAction for MenuAction {
    type Message = Message;

    fn message(&self) -> Self::Message {
        match self {
            MenuAction::MyProfile => Message::MyProfile,
            MenuAction::Drafts => Message::Compose(compose::Message::Drafts),
            MenuAction::Logout => Message::Dialog(DialogAction::Open(Dialog::Logout)),
            MenuAction::Conversations => Message::ToggleContextPage(ContextPage::Social(
                crate::features::social::Page::Conversations,
            )),
            MenuAction::Scheduled => Message::ToggleContextPage(ContextPage::Social(
                crate::features::social::Page::Scheduled,
            )),
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
