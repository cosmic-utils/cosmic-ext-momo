//! Real-time streaming of the authenticated user's events (new posts,
//! notifications, deletions), pushed into the relevant feature's message type.

use cosmic::iced::{stream, Subscription};
use futures_util::SinkExt;
use megalodon::streaming::Message as StreamMessage;

use crate::{app, client::Client, features::notifications, features::timeline};

pub fn stream_user_events(mastodon: Client) -> Subscription<app::Message> {
    Subscription::run_with(mastodon, |mastodon| {
        let mastodon = mastodon.clone();
        stream::channel(
            1,
            |output: futures_channel::mpsc::Sender<app::Message>| async move {
                let streaming = mastodon.user_streaming().await;

                streaming
                    .listen(Box::new(move |message| {
                        let mut output = output.clone();
                        Box::pin(async move {
                            let message = match message {
                                StreamMessage::Update(status) => {
                                    app::Message::Home(timeline::Message::IncomingStatus(status))
                                }
                                StreamMessage::Notification(notification) => {
                                    app::Message::Notifications(
                                        notifications::Message::PrependNotification(notification),
                                    )
                                }
                                StreamMessage::Delete(id) => app::Message::StatusDeleted(id),
                                StreamMessage::Conversation(_)
                                | StreamMessage::StatusUpdate(_)
                                | StreamMessage::Heartbeat() => return,
                            };
                            if let Err(err) = output.send(message).await {
                                tracing::warn!("failed to forward stream event: {err}");
                            }
                        })
                    }))
                    .await;

                std::future::pending().await
            },
        )
    })
}
