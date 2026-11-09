use super::*;
use crate::features::reading::filters::Filters;
use megalodon::entities::notification::NotificationType as Kind;
use serde_json::json;

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
fn inbox() -> Notifications {
    Notifications::new(Client::new("https://example.org".into(), None))
}

#[test]
fn groups_reactions_by_post_kind_and_day_without_losing_ids() {
    let mut cache = Cache::new();
    cache.reading.filters = Filters::ready(vec![]);
    let items = vec![
        notification("1", "favourite", "alice", "2026-01-01T12:00:00Z", true),
        notification("2", "favourite", "bob", "2026-01-01T12:01:00Z", true),
        notification("3", "favourite", "alice", "2026-01-01T12:02:00Z", true),
        notification("4", "reblog", "bob", "2026-01-01T12:03:00Z", true),
        notification("5", "favourite", "alice", "2026-01-03T12:00:00Z", true),
    ];
    let groups = model::groups(items.iter(), &cache, Tab::All, Filter::All);
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].ids, ["5"]);
    assert_eq!(groups[1].kind, Kind::Reblog);
    assert_eq!(groups[2].ids, ["1", "2", "3"]);
    assert_eq!(groups[2].actors.len(), 2);
    assert_eq!(model::actor_name(&groups[2].actors[0]), "alice");
}

#[test]
fn filters_fail_closed_for_posts_but_leave_follow_requests_actionable() {
    let mut cache = Cache::new();
    let items = vec![
        notification("1", "mention", "alice", "2026-01-01T12:00:00Z", true),
        notification("2", "follow_request", "bob", "2026-01-01T12:01:00Z", false),
    ];
    for filters in [Filters::Loading, Filters::Failed] {
        cache.reading.filters = filters;
        let groups = model::groups(items.iter(), &cache, Tab::All, Filter::All);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].ids, ["2"]);
    }
    cache.reading.filters = Filters::ready(vec![]);
    assert_eq!(
        model::groups(items.iter(), &cache, Tab::Mentions, Filter::All)[0].ids,
        ["1"]
    );
    assert_eq!(
        model::groups(items.iter(), &cache, Tab::Requests, Filter::All)[0].ids,
        ["2"]
    );
    assert!(model::groups(items.iter(), &cache, Tab::All, Filter::Favorites).is_empty());
}

#[test]
fn failed_mutations_keep_items_and_partial_dismiss_only_removes_successes() {
    let mut inbox = inbox();
    inbox.notifications.extend(["1".into(), "2".into()]);
    inbox.pending.extend(["1".into(), "2".into()]);
    let _ = inbox.update(Message::FollowFinished("1".into(), Err("offline".into())));
    assert!(inbox.notifications.contains(&"1".into()));
    assert!(!inbox.pending.contains("1"));
    assert_eq!(inbox.action_errors["1"], "offline");
    let _ = inbox.update(Message::Dismissed(vec![
        ("1".into(), Ok(())),
        ("2".into(), Err("offline".into())),
    ]));
    assert_eq!(inbox.notifications, ["2".to_string()]);
    assert!(inbox.dismissed.contains("1"));
    assert_eq!(inbox.action_errors["2"], "offline");
}

#[test]
fn clear_requires_confirmation_preserves_arrivals_and_rejects_stale_pages() {
    let mut inbox = inbox();
    inbox.notifications.push_back("1".into());
    let _ = inbox.update(Message::ClearAll);
    assert!(!inbox.clearing);
    let _ = inbox.update(Message::ConfirmClear);
    let previous_request = inbox.request;
    let _ = inbox.update(Message::ClearAll);
    assert!(inbox.clearing);
    let _ = inbox.update(Message::PageLoaded(
        previous_request,
        Ok(fetch::Page {
            notifications: vec![notification(
                "old-page",
                "follow",
                "alice",
                "2026-01-01T12:00:00Z",
                false,
            )],
            cursor: None,
            more: false,
        }),
    ));
    assert_eq!(inbox.notifications, ["1".to_string()]);
    let _ = inbox.update(Message::Cleared(vec!["1".into()], Err("offline".into())));
    assert_eq!(inbox.notifications.len(), 1);
    assert!(inbox.confirm_clear);
    inbox.new.push_front("2".into());
    let old_request = inbox.request;
    let _ = inbox.update(Message::Cleared(vec!["1".into()], Ok(())));
    assert!(inbox.notifications.is_empty());
    assert_eq!(inbox.new, ["2".to_string()]);
    let _ = inbox.update(Message::PageLoaded(
        old_request,
        Ok(fetch::Page {
            notifications: vec![notification(
                "1",
                "follow",
                "alice",
                "2026-01-01T12:00:00Z",
                false,
            )],
            cursor: Some("1".into()),
            more: true,
        }),
    ));
    assert!(inbox.notifications.is_empty());
}

#[test]
fn incoming_activity_waits_for_explicit_publish_and_is_deduplicated() {
    let mut inbox = inbox();
    let item = notification("1", "follow", "alice", "2026-01-01T12:00:00Z", false);
    let _ = inbox.update(Message::PrependNotification(item.clone()));
    let _ = inbox.update(Message::PrependNotification(item));
    assert!(inbox.notifications.is_empty());
    assert_eq!(inbox.new.len(), 1);
    let _ = inbox.update(Message::PublishNew);
    assert!(inbox.new.is_empty());
    assert_eq!(inbox.notifications, ["1".to_string()]);
}

#[test]
fn pagination_uses_next_link_rather_than_previous_or_numeric_id_assumptions() {
    assert_eq!(
        crate::client::next_cursor(
            r#"<https://example.org/api/v1/notifications?min_id=99>; rel="prev", <https://example.org/api/v1/notifications?max_id=opaque%2Bid>; rel="next""#
        ),
        Some("opaque+id".into())
    );
    assert_eq!(
        crate::client::next_cursor(
            r#"<https://example.org/api/v1/notifications?min_id=99>; rel="prev""#
        ),
        None
    );
    assert_eq!(crate::client::next_cursor("malformed"), None);
}

#[test]
fn desktop_alerts_never_include_private_post_or_content_warning_text() {
    let mut item = notification("private", "mention", "alice", "2026-01-01T12:00:00Z", true);
    let post = item.status.as_mut().unwrap();
    post.content = "secret private message".into();
    post.spoiler_text = "secret warning".into();
    post.visibility = megalodon::entities::status::StatusVisibility::Direct;
    let summary = super::desktop::summary(&item);
    assert!(summary.contains("alice"));
    assert!(!summary.contains("secret"));
}
