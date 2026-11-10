//! First-run setup and the reusable add-account flow.
//!
//! Requests carry both a wizard ID and a sequence number so a cancelled or
//! superseded authorization can never save credentials into a later wizard.
use cosmic::{iced::Length, widget, Element};
use megalodon::{entities::Account, oauth::AppData};

use crate::{
    cache::Cache,
    client::{Client, Session},
    fl,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    wizard: u64,
    sequence: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Welcome,
    Server,
    Authorize,
    Accounts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    InvalidServer,
    ServerUnavailable,
    Registration,
    Authorization,
    Verification,
    Keychain,
    Browser,
    Preferences,
}

impl Failure {
    pub fn text(self) -> String {
        match self {
            Self::InvalidServer => fl!("setup-invalid-server"),
            Self::ServerUnavailable => fl!("setup-server-unavailable"),
            Self::Registration => fl!("setup-registration-failed"),
            Self::Authorization => fl!("setup-authorization-failed"),
            Self::Verification => fl!("setup-verification-failed"),
            Self::Keychain => fl!("setup-keychain-failed"),
            Self::Browser => fl!("setup-browser-failed"),
            Self::Preferences => fl!("setup-preferences-failed"),
        }
    }
}

#[derive(Clone)]
pub enum Message {
    AddAccount,
    Browse,
    ServerChanged(String),
    Connect,
    Registered(Request, Result<(String, AppData), Failure>),
    CodeChanged(String),
    Authorize,
    OpenBrowser,
    Authorized(Request, Result<Client, Failure>),
    Verified(Request, Result<Account, Failure>),
    Saved(Request, Result<usize, Failure>),
    BrowserFailed,
    Back,
    SelectAccount(usize),
    Finish,
}

// Authorization responses and codes must never appear in debug logs.
impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OnboardingMessage")
    }
}

pub enum Effect {
    None,
    Register(Request, String),
    OpenBrowser(String),
    Exchange(Request, String, AppData, String),
    Verify(Request, Client),
    Save(Request, Client, Account),
    Finish(Option<usize>),
    Close,
}

pub struct State {
    pub step: Step,
    pub server: String,
    pub code: String,
    pub error: Option<Failure>,
    pub busy: bool,
    pub confirmed: Option<Account>,
    pub selected: Option<usize>,
    first_run: bool,
    request: Request,
    server_title: String,
    registration: Option<AppData>,
    client: Option<Client>,
    verified: Option<Account>,
}

impl State {
    pub fn new(wizard: u64, first_run: bool, server: String, selected: Option<usize>) -> Self {
        Self {
            step: if first_run {
                Step::Welcome
            } else {
                Step::Server
            },
            server,
            code: String::new(),
            error: None,
            busy: false,
            confirmed: None,
            selected,
            first_run,
            request: Request {
                wizard,
                sequence: 0,
            },
            server_title: String::new(),
            registration: None,
            client: None,
            verified: None,
        }
    }

    fn begin(&mut self) -> Request {
        self.request.sequence += 1;
        self.busy = true;
        self.error = None;
        self.request
    }

    fn accepts(&self, request: Request, step: Step) -> bool {
        self.busy && self.request == request && self.step == step
    }

    fn clear_authorization(&mut self) {
        self.request.sequence += 1;
        self.busy = false;
        self.error = None;
        self.code.clear();
        self.registration = None;
        self.client = None;
        self.verified = None;
    }

    pub fn update(&mut self, message: Message) -> Effect {
        match message {
            Message::AddAccount if !self.busy => {
                self.clear_authorization();
                self.step = Step::Server;
            }
            Message::Browse if self.step == Step::Welcome => return Effect::Finish(None),
            Message::ServerChanged(server) if self.step == Step::Server && !self.busy => {
                self.server = server;
                self.error = None;
            }
            Message::Connect if self.step == Step::Server && !self.busy => {
                match normalize_server(&self.server) {
                    Ok(server) => {
                        self.server = server.clone();
                        return Effect::Register(self.begin(), server);
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::Registered(request, result) if self.accepts(request, Step::Server) => {
                self.busy = false;
                match result {
                    Ok((title, registration)) => {
                        self.server_title = title;
                        let url = registration.url.clone();
                        self.registration = Some(registration);
                        self.step = Step::Authorize;
                        if let Some(url) = url {
                            return Effect::OpenBrowser(url);
                        }
                        self.error = Some(Failure::Registration);
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::CodeChanged(code) if self.step == Step::Authorize && !self.busy => {
                self.code = code;
                self.error = None;
            }
            Message::Authorize if self.step == Step::Authorize && !self.busy => {
                // Retain successful stages: retries never exchange a spent code again.
                if let Some(client) = self.client.clone() {
                    let request = self.begin();
                    return if let Some(account) = self.verified.clone() {
                        Effect::Save(request, client, account)
                    } else {
                        Effect::Verify(request, client)
                    };
                }
                if let Some(registration) = self.registration.clone() {
                    let code = self.code.trim().to_string();
                    if !code.is_empty() {
                        let request = self.begin();
                        return Effect::Exchange(request, self.server.clone(), registration, code);
                    }
                }
            }
            Message::Authorized(request, result) if self.accepts(request, Step::Authorize) => {
                match result {
                    Ok(client) => {
                        self.code.clear();
                        self.client = Some(client.clone());
                        return Effect::Verify(request, client);
                    }
                    Err(error) => {
                        self.busy = false;
                        self.error = Some(error);
                    }
                }
            }
            Message::Verified(request, result) if self.accepts(request, Step::Authorize) => {
                match result {
                    Ok(account) => {
                        if let Some(client) = self.client.clone() {
                            self.verified = Some(account.clone());
                            return Effect::Save(request, client, account);
                        }
                    }
                    Err(error) => {
                        self.busy = false;
                        self.error = Some(error);
                    }
                }
            }
            Message::Saved(request, result) if self.accepts(request, Step::Authorize) => {
                self.busy = false;
                match result {
                    Ok(index) => {
                        self.selected = Some(index);
                        self.confirmed = self.verified.take();
                        self.clear_authorization();
                        self.step = Step::Accounts;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Message::OpenBrowser if self.step == Step::Authorize => {
                if let Some(url) = self.registration.as_ref().and_then(|data| data.url.clone()) {
                    return Effect::OpenBrowser(url);
                }
            }
            Message::BrowserFailed if self.step == Step::Authorize => {
                self.error = Some(Failure::Browser)
            }
            Message::Back => {
                self.clear_authorization();
                match self.step {
                    Step::Authorize => self.step = Step::Server,
                    Step::Server if self.confirmed.is_some() => self.step = Step::Accounts,
                    Step::Server if self.first_run && self.selected.is_none() => {
                        self.step = Step::Welcome
                    }
                    Step::Accounts => return Effect::Finish(self.selected),
                    _ => return Effect::Close,
                }
            }
            Message::SelectAccount(index) if self.step == Step::Accounts => {
                self.selected = Some(index)
            }
            Message::Finish if self.step == Step::Accounts => return Effect::Finish(self.selected),
            _ => (),
        }
        Effect::None
    }

    pub fn view<'a>(&'a self, sessions: &'a [Session], cache: &'a Cache) -> Element<'a, Message> {
        let spacing = cosmic::theme::active().cosmic().spacing;
        let mut content = widget::column![]
            .spacing(spacing.space_m)
            .padding(spacing.space_l)
            .width(Length::Fill);
        match self.step {
            Step::Welcome => {
                content = content
                    .align_x(cosmic::iced::Alignment::Center)
                    .push(
                        widget::icon(widget::icon::from_svg_bytes(
                            include_bytes!("../../../res/icons/hicolor/scalable/apps/icon.svg")
                                .as_slice(),
                        ))
                        .size(96),
                    )
                    .push(
                        widget::text::title1(fl!("setup-welcome"))
                            .align_x(cosmic::iced::alignment::Horizontal::Center),
                    )
                    .push(
                        widget::text(fl!("setup-welcome-description"))
                            .align_x(cosmic::iced::alignment::Horizontal::Center),
                    )
                    .push(
                        widget::row![
                            widget::button::suggested(fl!("setup-add-account"))
                                .on_press(Message::AddAccount),
                            widget::button::standard(fl!("setup-browse")).on_press(Message::Browse),
                        ]
                        .spacing(spacing.space_s)
                        .align_y(cosmic::iced::Alignment::Center),
                    );
            }
            Step::Server => {
                let input = widget::text_input(fl!("server-url"), &self.server);
                let input = if self.busy {
                    input
                } else {
                    input
                        .on_input(Message::ServerChanged)
                        .on_submit(|_| Message::Connect)
                };
                content = content
                    .push(widget::text::title1(fl!("setup-server-title")))
                    .push(widget::text(fl!("setup-server-description")))
                    .push(input)
                    .push(widget::button::suggested(fl!("continue")).on_press_maybe(
                        (!self.busy && !self.server.trim().is_empty()).then_some(Message::Connect),
                    ));
            }
            Step::Authorize => {
                let input = widget::text_input(fl!("authorization-code"), &self.code);
                let input = if self.busy || self.client.is_some() {
                    input
                } else {
                    input
                        .on_input(Message::CodeChanged)
                        .on_submit(|_| Message::Authorize)
                };
                content = content
                    .push(widget::text::title1(fl!("confirm-authorization")))
                    .push(widget::text(format!(
                        "{} · {}",
                        self.server_title, self.server
                    )))
                    .push(widget::text(fl!("setup-authorize-description")))
                    .push(
                        widget::button::standard(fl!("setup-open-browser"))
                            .on_press(Message::OpenBrowser),
                    )
                    .push(input)
                    .push(
                        widget::button::suggested(if self.error.is_some() {
                            fl!("setup-retry")
                        } else {
                            fl!("confirm")
                        })
                        .on_press_maybe(
                            (!self.busy && (self.client.is_some() || !self.code.trim().is_empty()))
                                .then_some(Message::Authorize),
                        ),
                    );
            }
            Step::Accounts => {
                content = content.push(widget::text::title1(fl!("setup-account-ready")));
                if let Some(account) = &self.confirmed {
                    let avatar = cache
                        .handles
                        .get(&account.avatar)
                        .map(widget::image)
                        .unwrap_or_else(crate::cache::fallback_avatar);
                    content = content.push(avatar.width(72).height(72));
                    content = content.push(widget::text(account.display_name.clone()));
                }
                content = content.push(widget::text(fl!("setup-choose-account")));
                for (index, session) in sessions.iter().enumerate() {
                    content = content.push(widget::radio(
                        widget::text(session.label()),
                        index,
                        self.selected,
                        Message::SelectAccount,
                    ));
                }
                content = content.push(widget::text(fl!("setup-more-accounts"))).push(
                    widget::row![
                        widget::button::standard(fl!("setup-add-another"))
                            .on_press(Message::AddAccount),
                        widget::button::suggested(fl!("continue")).on_press(Message::Finish),
                    ]
                    .spacing(spacing.space_s)
                    .align_y(cosmic::iced::Alignment::Center),
                );
            }
        }
        if self.busy {
            content = content.push(
                widget::row![
                    widget::indeterminate_circular().size(24.),
                    widget::text(fl!("setup-working"))
                ]
                .spacing(spacing.space_s),
            );
        }
        if let Some(error) = self.error {
            content = content.push(widget::text(error.text()));
        }
        let container = widget::container(widget::scrollable(content))
            .max_width(600)
            .width(Length::Fill)
            .height(Length::Fill);
        if self.step == Step::Welcome {
            container.center_y(Length::Fill).into()
        } else {
            container.into()
        }
    }
}

/// Only an HTTPS server origin is accepted; never silently discard userinfo,
/// paths, queries, or fragments from a pasted URL.
pub fn normalize_server(input: &str) -> Result<String, Failure> {
    let input = input.trim();
    if input.is_empty() {
        return Err(Failure::InvalidServer);
    }
    let value = if input.contains("://") {
        input.to_string()
    } else {
        format!("https://{input}")
    };
    let url = reqwest::Url::parse(&value).map_err(|_| Failure::InvalidServer)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Failure::InvalidServer);
    }
    Ok(url.origin().ascii_serialization())
}

pub fn authorization_url_is_valid(server: &str, authorization: &str) -> bool {
    match (
        reqwest::Url::parse(server),
        reqwest::Url::parse(authorization),
    ) {
        (Ok(server), Ok(url)) => {
            url.origin() == server.origin() && url.username().is_empty() && url.password().is_none()
        }
        _ => false,
    }
}

/// Network errors are deliberately mapped to actionable messages without
/// retaining response bodies that could contain authorization credentials.
pub async fn register(request: Request, server: String) -> Message {
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let client = Client::new(server.clone(), None);
        let instance = client
            .get_instance()
            .await
            .map_err(|_| Failure::ServerUnavailable)?;
        let options = megalodon::megalodon::AppInputOptions {
            scopes: Some(
                ["read", "write", "follow"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
            ),
            redirect_uris: Some(megalodon::default::NO_REDIRECT.to_string()),
            ..Default::default()
        };
        let registration = client
            .register_app("Momo".into(), &options)
            .await
            .map_err(|_| Failure::Registration)?;
        if !registration
            .url
            .as_deref()
            .is_some_and(|url| authorization_url_is_valid(&server, url))
        {
            return Err(Failure::Registration);
        }
        Ok((instance.json.title, registration))
    })
    .await
    .unwrap_or(Err(Failure::ServerUnavailable));
    Message::Registered(request, result)
}

pub async fn exchange(
    request: Request,
    server: String,
    registration: AppData,
    code: String,
) -> Message {
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let client = Client::new(server.clone(), None);
        let token = client
            .fetch_access_token(
                registration.client_id,
                registration.client_secret,
                code,
                megalodon::default::NO_REDIRECT.to_string(),
            )
            .await
            .map_err(|_| Failure::Authorization)?;
        if token.access_token.is_empty() {
            return Err(Failure::Authorization);
        }
        Ok(Client::new(server, Some(token.access_token)))
    })
    .await
    .unwrap_or(Err(Failure::Authorization));
    Message::Authorized(request, result)
}

pub async fn verify(request: Request, client: Client) -> Message {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        client.verify_account_credentials(),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .map(|response| response.json)
    .ok_or(Failure::Verification);
    Message::Verified(request, result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Account {
        serde_json::from_value(serde_json::json!({
            "id": "42", "username": "alice", "acct": "alice", "display_name": "Alice",
            "locked": false, "created_at": "2020-01-01T00:00:00Z", "followers_count": 0,
            "following_count": 0, "statuses_count": 0, "note": "", "url": "https://example.org/@alice",
            "avatar": "https://example.org/avatar.png", "avatar_static": "", "header": "",
            "header_static": "", "emojis": [], "fields": [], "bot": false
        })).unwrap()
    }

    fn registration() -> AppData {
        let mut data = AppData::new(
            "1".into(),
            "Momo".into(),
            None,
            None,
            "client".into(),
            "secret".into(),
        );
        data.url = Some("https://example.org/oauth/authorize".into());
        data
    }

    fn authorizing() -> State {
        let mut state = State::new(1, true, "example.org".into(), None);
        state.update(Message::AddAccount);
        let Effect::Register(request, server) = state.update(Message::Connect) else {
            panic!("expected registration")
        };
        assert_eq!(server, "https://example.org");
        assert!(matches!(
            state.update(Message::Registered(
                request,
                Ok(("Example".into(), registration()))
            )),
            Effect::OpenBrowser(_)
        ));
        state
    }

    fn exchange_request(state: &mut State) -> Request {
        state.update(Message::CodeChanged(" code ".into()));
        let Effect::Exchange(request, _, _, code) = state.update(Message::Authorize) else {
            panic!("expected exchange")
        };
        assert_eq!(code, "code");
        request
    }

    #[test]
    fn accepts_server_origins_and_rejects_ambiguous_urls() {
        for (input, expected) in [
            (" example.org ", "https://example.org"),
            ("https://EXAMPLE.org/", "https://example.org"),
            ("example.org:8443", "https://example.org:8443"),
        ] {
            assert_eq!(normalize_server(input), Ok(expected.into()));
        }
        for invalid in [
            "",
            "http://example.org",
            "https://user:secret@example.org",
            "https://example.org/@alice",
            "example.org?token=secret",
            "example.org#fragment",
            "not a domain",
            "file:///tmp/test",
        ] {
            assert_eq!(
                normalize_server(invalid),
                Err(Failure::InvalidServer),
                "{invalid}"
            );
        }
        assert!(authorization_url_is_valid(
            "https://example.org",
            "https://example.org/oauth/authorize?client_id=1"
        ));
        assert!(!authorization_url_is_valid(
            "https://example.org",
            "https://other.org/oauth/authorize"
        ));
        assert!(!authorization_url_is_valid(
            "https://example.org",
            "javascript:alert(1)"
        ));
    }

    #[test]
    fn cancelled_and_superseded_requests_cannot_advance_setup() {
        let mut state = State::new(7, false, "example.org".into(), Some(0));
        let Effect::Register(old, _) = state.update(Message::Connect) else {
            panic!()
        };
        assert!(matches!(state.update(Message::Back), Effect::Close));
        let mut state = State::new(8, false, "example.org".into(), Some(0));
        let Effect::Register(current, _) = state.update(Message::Connect) else {
            panic!()
        };
        assert!(matches!(
            state.update(Message::Registered(old, Ok(("Old".into(), registration())))),
            Effect::None
        ));
        assert_eq!(state.step, Step::Server);
        assert!(state.busy);
        state.update(Message::Registered(
            current,
            Err(Failure::ServerUnavailable),
        ));
        assert!(!state.busy);
        assert_eq!(state.server, "https://example.org");
        let Effect::Register(retry, _) = state.update(Message::Connect) else {
            panic!()
        };
        assert_ne!(retry, current);
        state.update(Message::Registered(
            current,
            Ok(("Old".into(), registration())),
        ));
        assert_eq!(state.step, Step::Server);
    }

    #[test]
    fn failed_code_exchange_keeps_code_and_allows_retry() {
        let mut state = authorizing();
        let request = exchange_request(&mut state);
        state.update(Message::Authorized(request, Err(Failure::Authorization)));
        assert_eq!(state.code, " code ");
        assert_eq!(state.error, Some(Failure::Authorization));
        assert!(matches!(
            state.update(Message::Authorize),
            Effect::Exchange(_, _, _, _)
        ));
    }

    #[test]
    fn verification_and_keychain_failures_retry_without_reusing_code() {
        let mut state = authorizing();
        let request = exchange_request(&mut state);
        let client = Client::new("https://example.org".into(), Some("token".into()));
        assert!(matches!(
            state.update(Message::Authorized(request, Ok(client))),
            Effect::Verify(_, _)
        ));
        assert!(state.code.is_empty());
        state.update(Message::Verified(request, Err(Failure::Verification)));
        let Effect::Verify(request, _) = state.update(Message::Authorize) else {
            panic!("must reuse token")
        };
        assert!(matches!(
            state.update(Message::Verified(request, Ok(account()))),
            Effect::Save(_, _, _)
        ));
        state.update(Message::Saved(request, Err(Failure::Keychain)));
        assert_eq!(state.step, Step::Authorize);
        assert!(state.confirmed.is_none());
        let Effect::Save(request, _, verified) = state.update(Message::Authorize) else {
            panic!("must retry keychain only")
        };
        assert_eq!(verified.id, "42");
        state.update(Message::Saved(request, Ok(1)));
        assert_eq!(state.step, Step::Accounts);
        assert_eq!(state.selected, Some(1));
        assert!(state.registration.is_none() && state.client.is_none());
        state.update(Message::SelectAccount(0));
        assert!(matches!(
            state.update(Message::Finish),
            Effect::Finish(Some(0))
        ));
    }

    #[test]
    fn adding_another_account_preserves_the_first_confirmation() {
        let mut state = authorizing();
        let request = exchange_request(&mut state);
        state.update(Message::Authorized(
            request,
            Ok(Client::new(
                "https://example.org".into(),
                Some("token".into()),
            )),
        ));
        state.update(Message::Verified(request, Ok(account())));
        state.update(Message::Saved(request, Ok(0)));
        state.update(Message::AddAccount);
        assert_eq!(state.step, Step::Server);
        state.update(Message::Back);
        assert_eq!(state.step, Step::Accounts);
        assert_eq!(state.selected, Some(0));
        assert!(matches!(
            state.update(Message::Finish),
            Effect::Finish(Some(0))
        ));
    }

    #[test]
    fn guest_choice_is_explicit_and_back_preserves_existing_accounts() {
        let mut first_run = State::new(1, true, String::new(), None);
        assert_eq!(first_run.step, Step::Welcome);
        assert!(matches!(
            first_run.update(Message::Browse),
            Effect::Finish(None)
        ));
        first_run.update(Message::AddAccount);
        first_run.update(Message::Back);
        assert_eq!(first_run.step, Step::Welcome);
        let mut additional = State::new(2, false, String::new(), Some(0));
        assert_eq!(additional.step, Step::Server);
        assert!(matches!(additional.update(Message::Back), Effect::Close));
    }

    #[test]
    fn debug_output_never_contains_authorization_material() {
        assert!(
            !format!("{:?}", Message::CodeChanged("sensitive-code".into()))
                .contains("sensitive-code")
        );
        assert!(!format!(
            "{:?}",
            Message::Registered(
                Request {
                    wizard: 1,
                    sequence: 1
                },
                Ok(("Server".into(), registration()))
            )
        )
        .contains("secret"));
    }
}
