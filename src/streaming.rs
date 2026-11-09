//! Cancellable streaming with bounded reconnect backoff and explicit recovery.
//! Tokens travel only in the Authorization header, never in URLs or logs.
use std::time::Duration;

use cosmic::iced::{stream, Subscription};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest, http::header::AUTHORIZATION, Message as Frame,
};

use crate::{
    app,
    client::Client,
    features::{notifications, timeline},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Connecting,
    Connected,
    Reconnecting,
    Unauthorized,
}

fn delay(attempt: u32) -> Duration {
    Duration::from_secs((1u64 << attempt.min(6)).min(60))
}

fn tls_connector() -> Option<tokio_tungstenite::Connector> {
    // Reqwest and libcosmic can enable different Rustls providers. Select one
    // explicitly rather than relying on Rustls's ambiguous global default.
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().certs {
        let _ = roots.add(cert);
    }
    let config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .ok()?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Some(tokio_tungstenite::Connector::Rustls(std::sync::Arc::new(
        config,
    )))
}

fn endpoint(base: &str) -> Option<reqwest::Url> {
    let mut url = reqwest::Url::parse(base).ok()?;
    match url.scheme() {
        "https" | "wss" => url.set_scheme("wss").ok()?,
        _ => return None,
    }
    if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    url.set_path("/api/v1/streaming");
    url.set_query(Some("stream=user"));
    url.set_fragment(None);
    Some(url)
}

#[derive(serde::Deserialize)]
struct Event {
    event: String,
    #[serde(default)]
    payload: String,
}

// Megalodon's public entities require quote policy even on pre-4.5 servers.
// Its internal Mastodon decoder supplies the same default; preserve that here.
fn payload(text: &str) -> Option<serde_json::Value> {
    fn normalize(value: &mut serde_json::Value) {
        if let Some(object) = value.as_object_mut() {
            if object.contains_key("content") && object.contains_key("media_attachments") {
                if object
                    .get("quote_approval")
                    .is_none_or(serde_json::Value::is_null)
                {
                    object.insert(
                        "quote_approval".into(),
                        serde_json::to_value(megalodon::entities::QuoteApproval::default())
                            .unwrap(),
                    );
                }
            }
            if let Some(kind) = object.get_mut("type") {
                match kind.as_str() {
                    Some("poll") => *kind = "poll_expired".into(),
                    Some("admin.sign_up") => *kind = "admin_signup".into(),
                    Some("admin.report") => *kind = "admin_report".into(),
                    _ => (),
                }
            }
            for key in ["reblog", "status"] {
                if let Some(nested) = object.get_mut(key) {
                    normalize(nested);
                }
            }
        }
    }
    let mut value = serde_json::from_str(text).ok()?;
    normalize(&mut value);
    Some(value)
}

fn event(text: &str) -> Option<app::Message> {
    let event: Event = serde_json::from_str(text).ok()?;
    match event.event.as_str() {
        "update" | "status.update" => {
            let status: megalodon::entities::Status =
                serde_json::from_value(payload(&event.payload)?).ok()?;
            if event.event == "update" {
                Some(app::Message::Home(timeline::Message::IncomingStatus(
                    status,
                )))
            } else {
                Some(app::Message::StatusUpdated(status))
            }
        }
        "notification" => {
            let notification: megalodon::entities::Notification =
                serde_json::from_value(payload(&event.payload)?).ok()?;
            Some(app::Message::Notifications(
                notifications::Message::PrependNotification(notification),
            ))
        }
        "delete" if !event.payload.is_empty() => Some(app::Message::StatusDeleted(event.payload)),
        "conversation" => Some(app::Message::Social(
            crate::features::social::Message::ConversationUpdated,
        )),
        "filters_changed" => Some(app::Message::ReloadFilters),
        "notifications_merged" => {
            Some(app::Message::Notifications(notifications::Message::Refresh))
        }
        _ => None,
    }
}

pub fn stream_user_events(client: Client) -> Subscription<app::Message> {
    Subscription::run_with(client, |client| {
        let client = client.clone();
        stream::channel(
            16,
            |mut output: futures_channel::mpsc::Sender<app::Message>| async move {
                let mut attempt = 0;
                if output
                    .send(app::Message::StreamState(State::Connecting))
                    .await
                    .is_err()
                {
                    return;
                }
                loop {
                    // Rediscover on each attempt: server streaming hosts can change.
                    let advertised =
                        tokio::time::timeout(Duration::from_secs(30), client.get_instance()).await;
                    let base = match advertised {
                        Ok(Ok(response)) => response
                            .json
                            .urls
                            .map(|urls| urls.streaming_api)
                            .unwrap_or_else(|| client.base_url.clone()),
                        Ok(Err(error)) if crate::client::unauthorized(&error) => {
                            let _ = output
                                .send(app::Message::StreamState(State::Unauthorized))
                                .await;
                            std::future::pending::<()>().await;
                            return;
                        }
                        _ => client.base_url.clone(),
                    };
                    let connection = async {
                        let url = endpoint(&base).ok_or(())?;
                        let mut request = url.as_str().into_client_request().map_err(|_| ())?;
                        let mut auth =
                            format!("Bearer {}", client.token.as_deref().unwrap_or_default())
                                .parse()
                                .map_err(|_| ())?;
                        request.headers_mut().insert(AUTHORIZATION, {
                            tokio_tungstenite::tungstenite::http::HeaderValue::set_sensitive(
                                &mut auth, true,
                            );
                            auth
                        });
                        let connector = tls_connector().ok_or(())?;
                        match tokio_tungstenite::connect_async_tls_with_config(
                            request,
                            None,
                            false,
                            Some(connector),
                        )
                        .await
                        {
                            Ok((socket, _)) => Ok(Some(socket)),
                            Err(tokio_tungstenite::tungstenite::Error::Http(response))
                                if response.status().as_u16() == 401 =>
                            {
                                Ok(None)
                            }
                            Err(_) => Err(()),
                        }
                    };
                    match tokio::time::timeout(Duration::from_secs(30), connection).await {
                        Ok(Ok(None)) => {
                            let _ = output
                                .send(app::Message::StreamState(State::Unauthorized))
                                .await;
                            std::future::pending::<()>().await;
                            return;
                        }
                        Ok(Ok(Some(mut socket))) => {
                            let connected = std::time::Instant::now();
                            if output
                                .send(app::Message::StreamState(State::Connected))
                                .await
                                .is_err()
                            {
                                return;
                            }
                            loop {
                                // Mastodon sends heartbeat pings. A dead socket must not hang forever.
                                match tokio::time::timeout(Duration::from_secs(90), socket.next())
                                    .await
                                {
                                    Ok(Some(Ok(Frame::Text(text)))) => {
                                        if let Some(message) = event(&text) {
                                            if output.send(message).await.is_err() {
                                                return;
                                            }
                                        }
                                    }
                                    Ok(Some(Ok(Frame::Ping(bytes)))) => {
                                        if !matches!(
                                            tokio::time::timeout(
                                                Duration::from_secs(10),
                                                socket.send(Frame::Pong(bytes))
                                            )
                                            .await,
                                            Ok(Ok(()))
                                        ) {
                                            break;
                                        }
                                    }
                                    Ok(Some(Ok(Frame::Pong(_)))) => (),
                                    _ => break,
                                }
                            }
                            if connected.elapsed() >= Duration::from_secs(60) {
                                attempt = 0;
                            }
                        }
                        _ => (),
                    }
                    // Some servers accept the WebSocket upgrade, then close it for
                    // a revoked token. Check REST credentials before retrying forever.
                    if matches!(tokio::time::timeout(Duration::from_secs(30), client.verify_account_credentials()).await,
                        Ok(Err(error)) if crate::client::unauthorized(&error))
                    {
                        let _ = output
                            .send(app::Message::StreamState(State::Unauthorized))
                            .await;
                        std::future::pending::<()>().await;
                        return;
                    }
                    if output
                        .send(app::Message::StreamState(State::Reconnecting))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    tokio::time::sleep(delay(attempt)).await;
                    attempt = attempt.saturating_add(1);
                }
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mastodon_wire_posts_and_edits_are_decoded_without_a_quote_policy() {
        let account = serde_json::json!({
            "id":"a1", "username":"alice", "acct":"alice", "display_name":"Alice", "locked":false,
            "created_at":"2020-01-01T00:00:00Z", "followers_count":0, "following_count":0,
            "statuses_count":0, "note":"", "url":"https://example.org/@alice", "avatar":"", "avatar_static":"",
            "header":"", "header_static":"", "emojis":[], "fields":[], "bot":false
        });
        let post = serde_json::json!({
            "id":"p1","uri":"https://example.org/posts/p1","account":account,"content":"<p>Edited</p>",
            "created_at":"2020-01-01T00:00:00Z","emojis":[],"replies_count":0,"reblogs_count":0,
            "favourites_count":0,"sensitive":false,"spoiler_text":"","visibility":"public",
            "media_attachments":[],"mentions":[],"tags":[]
        });
        let wire = |kind, payload: serde_json::Value| {
            serde_json::json!({"event":kind,"payload":payload.to_string()}).to_string()
        };
        assert!(
            matches!(event(&wire("update", post.clone())), Some(app::Message::Home(timeline::Message::IncomingStatus(status))) if status.id == "p1")
        );
        assert!(
            matches!(event(&wire("status.update", post.clone())), Some(app::Message::StatusUpdated(status)) if status.content.contains("Edited"))
        );
        let notification = serde_json::json!({"id":"n1","account":account,"type":"mention","created_at":"2020-01-01T00:00:00Z","status":post});
        assert!(
            matches!(event(&wire("notification", notification)), Some(app::Message::Notifications(notifications::Message::PrependNotification(notification))) if notification.status.as_ref().is_some_and(|status| status.id == "p1"))
        );
    }
    #[test]
    fn older_server_payloads_receive_missing_policy_and_notification_type_defaults() {
        let value = payload(r#"{"content":"Hello","media_attachments":[],"reblog":{"content":"Old","media_attachments":[]}}"#).unwrap();
        assert_eq!(value["quote_approval"]["current_user"], "unknown");
        assert_eq!(
            value["reblog"]["quote_approval"]["automatic"],
            serde_json::json!([])
        );
        assert_eq!(
            payload(r#"{"type":"poll"}"#).unwrap()["type"],
            "poll_expired"
        );
        assert_eq!(
            payload(r#"{"type":"admin.sign_up"}"#).unwrap()["type"],
            "admin_signup"
        );
    }
    #[test]
    fn endpoint_keeps_tokens_out_of_urls_and_requires_tls() {
        let url = endpoint("https://stream.example.org/old?access_token=secret").unwrap();
        assert_eq!(
            url.as_str(),
            "wss://stream.example.org/api/v1/streaming?stream=user"
        );
        for bad in [
            "http://example.org",
            "ws://example.org",
            "wss://user:secret@example.org",
            "invalid",
        ] {
            assert!(endpoint(bad).is_none());
        }
    }
    #[test]
    fn backoff_is_bounded_and_events_are_routed_without_panics() {
        assert_eq!(delay(0).as_secs(), 1);
        assert_eq!(delay(99).as_secs(), 60);
        assert!(
            matches!(event(r#"{"event":"delete","payload":"abc"}"#), Some(app::Message::StatusDeleted(id)) if id == "abc")
        );
        assert!(matches!(
            event(r#"{"event":"filters_changed"}"#),
            Some(app::Message::ReloadFilters)
        ));
        assert!(event(r#"{"event":"status.update","payload":"invalid"}"#).is_none());
        assert!(event("invalid").is_none());
    }
}
