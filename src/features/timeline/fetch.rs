use cosmic::iced::{stream, Subscription};
use futures_util::SinkExt;
use megalodon::megalodon::{
    GetBookmarksInputOptions, GetFavouritesInputOptions, GetHomeTimelineInputOptions,
    GetListTimelineInputOptions, GetLocalTimelineInputOptions, GetPublicTimelineInputOptions,
    GetTagTimelineInputOptions,
};

use crate::client::Client;

use super::{Message, TimelineKind};

const PAGE_SIZE: u32 = 20;

pub fn timeline(
    mastodon: Client,
    kind: TimelineKind,
    max_id: Option<String>,
    request: u64,
) -> Subscription<Message> {
    Subscription::run_with(
        (mastodon, kind, max_id, request),
        |(mastodon, kind, max_id, request)| {
            let request = *request;
            let mastodon = mastodon.clone();
            let kind = kind.clone();
            let max_id = max_id.clone();
            stream::channel(
                1,
                move |mut output: futures_channel::mpsc::Sender<Message>| async move {
                    let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                        match &kind {
                            TimelineKind::Home => {
                                let options = GetHomeTimelineInputOptions {
                                    max_id,
                                    limit: Some(PAGE_SIZE),
                                    ..Default::default()
                                };
                                mastodon.get_home_timeline(Some(&options)).await.map(page)
                            }
                            TimelineKind::Local => {
                                let options = GetLocalTimelineInputOptions {
                                    max_id,
                                    limit: Some(PAGE_SIZE),
                                    ..Default::default()
                                };
                                mastodon.get_local_timeline(Some(&options)).await.map(page)
                            }
                            TimelineKind::Public => {
                                let options = GetPublicTimelineInputOptions {
                                    max_id,
                                    limit: Some(PAGE_SIZE),
                                    ..Default::default()
                                };
                                mastodon.get_public_timeline(Some(&options)).await.map(page)
                            }
                            TimelineKind::Federated => {
                                // megalodon has no "remote only" flag on the public timeline endpoint,
                                // so federated posts are derived by filtering out local ones: a status's
                                // account `acct` only carries an `@instance` suffix for remote accounts.
                                let options = GetPublicTimelineInputOptions {
                                    max_id,
                                    limit: Some(PAGE_SIZE),
                                    ..Default::default()
                                };
                                mastodon
                                    .get_public_timeline(Some(&options))
                                    .await
                                    .map(|response| {
                                        let mut page = page(response);
                                        page.statuses
                                            .retain(|status| status.account.acct.contains('@'));
                                        page
                                    })
                            }
                            TimelineKind::Favorites => {
                                let options = GetFavouritesInputOptions {
                                    max_id,
                                    limit: Some(PAGE_SIZE),
                                    ..Default::default()
                                };
                                mastodon.get_favourites(Some(&options)).await.map(page)
                            }
                            TimelineKind::Bookmarks => {
                                let options = GetBookmarksInputOptions {
                                    max_id,
                                    limit: Some(PAGE_SIZE),
                                    ..Default::default()
                                };
                                mastodon.get_bookmarks(Some(&options)).await.map(page)
                            }
                            TimelineKind::Tag(tag) => {
                                let options = GetTagTimelineInputOptions {
                                    max_id,
                                    limit: Some(PAGE_SIZE),
                                    ..Default::default()
                                };
                                mastodon
                                    .get_tag_timeline(tag.clone(), Some(&options))
                                    .await
                                    .map(page)
                            }
                            TimelineKind::List(id) => {
                                let options = GetListTimelineInputOptions {
                                    max_id,
                                    limit: Some(PAGE_SIZE),
                                    ..Default::default()
                                };
                                mastodon
                                    .get_list_timeline(id.clone(), Some(&options))
                                    .await
                                    .map(page)
                            }
                        }
                    })
                    .await;
                    let result = result
                        .ok()
                        .and_then(Result::ok)
                        .ok_or_else(|| crate::fl!("feed-load-error"));
                    let _ = output.send(Message::PageLoaded(request, result)).await;

                    std::future::pending().await
                },
            )
        },
    )
}

#[derive(Debug, Clone)]
pub struct Page {
    pub statuses: Vec<megalodon::entities::Status>,
    pub cursor: Option<String>,
    pub more: bool,
}
fn page(response: megalodon::response::Response<Vec<megalodon::entities::Status>>) -> Page {
    let next = response
        .header
        .get("link")
        .and_then(|value| value.to_str().ok())
        .and_then(crate::client::next_cursor);
    let more = next.is_some()
        || (response.header.get("link").is_none() && response.json.len() >= PAGE_SIZE as usize);
    let cursor = next.or_else(|| response.json.last().map(|post| post.id.clone()));
    Page {
        statuses: response.json,
        cursor,
        more,
    }
}
