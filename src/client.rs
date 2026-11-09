// SPDX-License-Identifier: {{LICENSE}}

use std::sync::Arc;

use megalodon::Megalodon;

/// Wraps a megalodon client together with the connection details we need to
/// track ourselves (megalodon's clients don't expose their base URL or token).
#[derive(Clone)]
pub struct Client {
    pub base_url: String,
    pub token: Option<String>,
    pub account_id: String,
    inner: Arc<Box<dyn Megalodon + Send + Sync>>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.base_url)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl std::hash::Hash for Client {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.base_url.hash(state);
        self.token.hash(state);
    }
}

impl PartialEq for Client {
    fn eq(&self, other: &Self) -> bool {
        self.base_url == other.base_url && self.token == other.token
    }
}

impl Eq for Client {}

impl Client {
    pub fn new(base_url: String, token: Option<String>) -> Self {
        let inner = megalodon::generator(
            megalodon::SNS::Mastodon,
            base_url.clone(),
            token.clone(),
            Some("toot".to_string()),
        )
        .expect("failed to create megalodon client");
        Self {
            base_url,
            token,
            account_id: String::new(),
            inner: Arc::new(inner),
        }
    }

    pub fn from_session(session: &Session) -> Self {
        let mut client = Self::new(session.base_url.clone(), Some(session.token.clone()));
        client.account_id = session.account_id.clone();
        client
    }

    pub fn cache_key(&self) -> String {
        // Unverified legacy sessions never read the old instance-wide private cache.
        format!(
            "{}#{}",
            self.base_url,
            if self.account_id.is_empty() {
                "public"
            } else {
                &self.account_id
            }
        )
    }

    pub fn is_authenticated(&self) -> bool {
        self.token.as_ref().is_some_and(|token| !token.is_empty())
    }
}

impl std::ops::Deref for Client {
    type Target = dyn Megalodon + Send + Sync;

    fn deref(&self) -> &Self::Target {
        &**self.inner
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Session {
    pub base_url: String,
    pub token: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub acct: String,
    #[serde(default)]
    pub preferences: Option<crate::config::TootConfig>,
}

/// All accounts saved in the keychain, and which one is active. Persisted as
/// a single JSON blob under one keychain entry.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Sessions {
    pub active: usize,
    pub sessions: Vec<Session>,
}

impl Sessions {
    /// Parse the stored keychain payload, falling back to interpreting it as
    /// a single legacy (pre-multi-account) session if it isn't a [`Sessions`].
    pub fn parse(data: &str) -> Option<Sessions> {
        if let Ok(sessions) = serde_json::from_str::<Sessions>(data) {
            return Some(sessions);
        }
        serde_json::from_str::<Session>(data)
            .ok()
            .map(|session| Sessions {
                active: 0,
                sessions: vec![session],
            })
    }

    pub fn active_session(&self) -> Option<&Session> {
        self.sessions.get(self.active)
    }

    /// Add a session, or replace an existing one for the same instance, and
    /// make it the active one.
    pub fn upsert_active(&mut self, session: Session) {
        if let Some(index) = self.sessions.iter().position(|existing| {
            existing.base_url == session.base_url
                && (existing.account_id == session.account_id || existing.token == session.token)
        }) {
            self.sessions[index] = session;
            self.active = index;
        } else {
            self.sessions.push(session);
            self.active = self.sessions.len() - 1;
        }
    }

    /// Remove the session at `index`. If it was the active one, activate the
    /// previous session (or the new one at the same position), if any remain.
    pub fn remove(&mut self, index: usize) {
        if index >= self.sessions.len() {
            return;
        }
        self.sessions.remove(index);
        if index < self.active {
            self.active -= 1;
        }
        if self.sessions.is_empty() {
            self.active = 0;
        } else {
            self.active = self.active.min(self.sessions.len() - 1);
        }
    }
}

impl Session {
    pub fn label(&self) -> String {
        let host = reqwest::Url::parse(&self.base_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_default();
        let acct = if self.acct.contains('@') {
            self.acct.clone()
        } else if self.acct.is_empty() {
            self.account_id.clone()
        } else {
            format!("{}@{}", self.acct, host)
        };
        format!("@{} · {}", acct, self.base_url)
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("base_url", &self.base_url)
            .field("account_id", &self.account_id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(id: &str) -> Session {
        Session {
            base_url: "https://example.org".into(),
            token: id.into(),
            account_id: id.into(),
            acct: id.into(),
            preferences: None,
        }
    }
    #[test]
    fn same_server_accounts_remain_distinct() {
        let mut sessions = Sessions::default();
        sessions.upsert_active(session("1"));
        sessions.upsert_active(session("2"));
        assert_eq!(sessions.sessions.len(), 2);
        assert_ne!(
            Client::from_session(&sessions.sessions[0]).cache_key(),
            Client::from_session(&sessions.sessions[1]).cache_key()
        );
        sessions.upsert_active(session("1"));
        assert_eq!(sessions.sessions.len(), 2);
    }
    #[test]
    fn removing_earlier_account_preserves_active_identity() {
        let mut sessions = Sessions::default();
        for id in ["1", "2", "3"] {
            sessions.upsert_active(session(id));
        }
        sessions.remove(0);
        assert_eq!(sessions.active_session().unwrap().account_id, "3");
    }
    #[test]
    fn legacy_session_is_readable_without_logging_token() {
        let sessions =
            Sessions::parse(r#"{"base_url":"https://example.org","token":"secret"}"#).unwrap();
        assert!(sessions.active_session().unwrap().account_id.is_empty());
        assert!(!format!("{sessions:?}").contains("secret"));
    }
    #[test]
    fn saved_preferences_from_before_onboarding_are_compatible() {
        let preferences: crate::config::TootConfig = serde_json::from_value(serde_json::json!({
            "server": "example.org", "hide_boosts": true, "hide_replies": false,
            "feed_density": "Compact", "theme_mode": "Dark"
        }))
        .unwrap();
        assert!(!preferences.onboarding_completed);
        assert!(preferences.hide_boosts);
        let mut first = session("1");
        first.acct = "alice".into();
        assert_eq!(first.label(), "@alice@example.org · https://example.org");
        first.acct = "alice@example.org".into();
        assert_eq!(first.label(), "@alice@example.org · https://example.org");
    }
}
