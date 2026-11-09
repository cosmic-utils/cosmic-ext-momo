use super::*;
use megalodon::entities::Notification;
use serde_json::json;
fn post(id: &str) -> Status {
    let mut post = notification(id, "mention", "alice", "2026-01-01T12:00:00Z", true)
        .status
        .unwrap();
    post.id = id.into();
    post
}
fn feed() -> Timeline {
    Timeline::new(
        Client::new("https://example.org".into(), None),
        TimelineKind::Public,
    )
}
fn page(ids: &[&str], cursor: &str, more: bool) -> fetch::Page {
    fetch::Page {
        statuses: ids.iter().map(|id| post(id)).collect(),
        cursor: Some(cursor.into()),
        more,
    }
}
#[test]
fn failures_preserve_posts_and_retry_the_same_page_with_a_new_request() {
    let mut feed = feed();
    let _ = feed.update(Message::PageLoaded(0, Ok(page(&["1"], "cursor", true))));
    let _ = feed.update(Message::LoadMore);
    let failed = feed.request;
    let _ = feed.update(Message::PageLoaded(failed, Err("offline".into())));
    assert_eq!(feed.statuses, ["1".to_string()]);
    assert_eq!(feed.max_id.as_deref(), Some("cursor"));
    assert!(!feed.loading);
    let _ = feed.update(Message::Retry);
    assert!(feed.loading && feed.older);
    assert!(feed.request > failed);
    assert!(feed.error.is_none());
}
#[test]
fn refresh_restarts_pagination_and_stale_pages_cannot_replace_it() {
    let mut feed = feed();
    let _ = feed.update(Message::PageLoaded(0, Ok(page(&["1"], "cursor", true))));
    let _ = feed.update(Message::Refresh);
    let _ = feed.update(Message::PageLoaded(0, Ok(page(&["stale"], "old", false))));
    assert_eq!(feed.statuses, ["1".to_string()]);
    let _ = feed.update(Message::PageLoaded(
        feed.request,
        Ok(page(&["2"], "new-cursor", true)),
    ));
    assert_eq!(feed.statuses, ["2".to_string()]);
    assert_eq!(feed.max_id.as_deref(), Some("new-cursor"));
    let _ = feed.update(Message::LoadMore);
    let _ = feed.update(Message::PageLoaded(
        feed.request,
        Ok(page(&["2", "3"], "older-cursor", false)),
    ));
    assert_eq!(feed.statuses, ["2".to_string(), "3".to_string()]);
    assert!(!feed.more);
}
#[test]
fn stream_posts_wait_for_publish_but_own_posts_are_immediate_and_deletions_remove_both() {
    let mut feed = feed();
    let _ = feed.update(Message::IncomingStatus(post("1")));
    let _ = feed.update(Message::IncomingStatus(post("1")));
    assert!(feed.statuses.is_empty());
    assert_eq!(feed.incoming.len(), 1);
    let _ = feed.update(Message::PrependStatus(post("1")));
    assert!(feed.incoming.is_empty());
    assert_eq!(feed.statuses, ["1".to_string()]);
    let _ = feed.update(Message::IncomingStatus(post("2")));
    let _ = feed.update(Message::PublishNew);
    assert_eq!(feed.statuses, ["2".to_string(), "1".to_string()]);
    let _ = feed.update(Message::IncomingStatus(post("3")));
    let _ = feed.update(Message::DeleteStatus("3".into()));
    let _ = feed.update(Message::DeleteStatus("2".into()));
    assert!(feed.incoming.is_empty());
    assert_eq!(feed.statuses, ["1".to_string()]);
}
#[test]
fn account_reset_clears_buffered_posts_errors_and_invalidates_previous_requests() {
    let mut feed = feed();
    let _ = feed.update(Message::IncomingStatus(post("1")));
    let previous = feed.request;
    feed.reset(Client::new("https://another.example.org".into(), None));
    let _ = feed.update(Message::PageLoaded(
        previous,
        Ok(page(&["1"], "cursor", true)),
    ));
    assert!(feed.statuses.is_empty() && feed.incoming.is_empty());
    assert!(!feed.has_loaded && feed.error.is_none());
}
#[test]
fn repeated_pagination_cursor_stops_offering_duplicate_pages() {
    let mut feed = feed();
    let _ = feed.update(Message::PageLoaded(0, Ok(page(&["1"], "cursor", true))));
    let _ = feed.update(Message::LoadMore);
    let _ = feed.update(Message::PageLoaded(
        feed.request,
        Ok(page(&["1"], "cursor", true)),
    ));
    assert!(!feed.more);
    assert_eq!(feed.statuses.len(), 1);
}
fn notification(id: &str, kind: &str, actor: &str, date: &str, post: bool) -> Notification {
    let account = json!({
        "id":actor,"username":actor,"acct":actor,"display_name":"","locked":false,
        "created_at":"2020-01-01T00:00:00Z","followers_count":0,"following_count":0,
        "statuses_count":0,"note":"","url":"https://example.org/@alice","avatar":"",
        "avatar_static":"","header":"","header_static":"","emojis":[],"fields":[],"bot":false
    });
    let status = post.then(|| json!({
        "id":"s1","uri":"https://example.org/posts/s1","account":account,"content":"<p>Hello</p>",
        "created_at":"2020-01-01T00:00:00Z","emojis":[],"replies_count":0,"reblogs_count":0,
        "favourites_count":0,"sensitive":false,"spoiler_text":"","visibility":"public",
        "media_attachments":[],"mentions":[],"tags":[],
        "quote_approval":{"automatic":[],"manual":[],"current_user":"unknown"}
    }));
    serde_json::from_value(
        json!({"id":id,"type":kind,"account":account,"created_at":date,"status":status}),
    )
    .unwrap()
}

#[test]
fn a_late_page_cannot_restore_a_stream_deleted_post() {
    let mut feed = feed();
    let _ = feed.update(Message::IncomingStatus(post("1")));
    let _ = feed.update(Message::DeleteStatus("1".into()));
    let _ = feed.update(Message::PageLoaded(
        0,
        Ok(page(&["1", "2"], "cursor", true)),
    ));
    let _ = feed.update(Message::IncomingStatus(post("1")));
    assert_eq!(feed.statuses, ["2".to_string()]);
    assert!(feed.incoming.is_empty());
}
