use super::Message;
use crate::{
    client::{next_cursor, Client},
    fl,
};
use cosmic::iced::{stream, Subscription};
use futures_util::SinkExt;
use megalodon::{entities::Notification, megalodon::GetNotificationsInputOptions};

#[derive(Debug, Clone)]
pub struct Page {
    pub notifications: Vec<Notification>,
    pub cursor: Option<String>,
    pub more: bool,
}

pub fn timeline(client: Client, request: u64, cursor: Option<String>) -> Subscription<Message> {
    Subscription::run_with((client, request, cursor), |(client, request, cursor)| {
        let client = client.clone();
        let request = *request;
        let cursor = cursor.clone();
        stream::channel(
            1,
            move |mut output: futures_channel::mpsc::Sender<Message>| async move {
                let result = crate::client::within_timeout(
                    client.get_notifications(Some(&GetNotificationsInputOptions {
                        limit: Some(30),
                        max_id: cursor,
                        ..Default::default()
                    })),
                    fl!("activity-load-error"),
                )
                .await
                .map(|response| {
                    let next = response
                        .header
                        .get("link")
                        .and_then(|value| value.to_str().ok())
                        .and_then(next_cursor);
                    let more = next.is_some()
                        || (response.header.get("link").is_none() && response.json.len() == 30);
                    let cursor = next.or_else(|| {
                        response
                            .json
                            .last()
                            .map(|notification| notification.id.clone())
                    });
                    Page {
                        notifications: response.json,
                        cursor,
                        more,
                    }
                });
                let _ = output.send(Message::PageLoaded(request, result)).await;
                std::future::pending().await
            },
        )
    })
}
