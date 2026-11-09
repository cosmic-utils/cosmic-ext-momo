use super::*;
use crate::cache::Cache;
use megalodon::entities::{Account, Attachment, Poll, Status};
use serde_json::json;

fn account() -> Account {
    serde_json::from_value(json!({
        "id":"a1", "username":"alice", "acct":"alice", "display_name":"Alice", "locked":false,
        "created_at":"2020-01-01T00:00:00Z", "followers_count":0, "following_count":0,
        "statuses_count":0, "note":"", "url":"https://example.org/@alice", "avatar":"", "avatar_static":"",
        "header":"", "header_static":"", "emojis":[], "fields":[], "bot":false
    })).unwrap()
}
fn post(id: &str, content: &str) -> Status {
    serde_json::from_value(json!({
        "id":id,"uri":format!("https://example.org/posts/{id}"),"account":account(),"content":content,
        "created_at":"2020-01-01T00:00:00Z","emojis":[],"replies_count":0,"reblogs_count":0,
        "favourites_count":0,"sensitive":false,"spoiler_text":"","visibility":"public",
        "media_attachments":[],"mentions":[],"tags":[],"quote_approval":{"automatic":[],"manual":[],"current_user":"unknown"}
    })).unwrap()
}
fn filter(action: &str, keyword: &str, context: &str) -> filters::Filter {
    serde_json::from_value(
        json!({"title":"My filter","filter_action":action,"context":[context],"expires_at":null,
        "keywords":[{"keyword":keyword,"whole_word":true}],"statuses":[]}),
    )
    .unwrap()
}
fn poll(multiple: bool) -> Poll {
    serde_json::from_value(json!({"id":"p1","expires_at":"2099-01-01T00:00:00Z","expired":false,
        "multiple":multiple,"votes_count":3,"options":[{"title":"One","votes_count":1},{"title":"Two","votes_count":2}],"voted":false,"emojis":[]})).unwrap()
}

#[test]
fn html_preserves_links_entities_and_selectable_text_without_executable_schemes() {
    let lines = html::document(
        r#"<p>Hello &amp; <a href="https://example.org/@alice"><span>@alice</span></a>!</p><p><a href="javascript:alert(1)">unsafe</a></p>"#,
    );
    let fragments: Vec<_> = lines.iter().flatten().collect();
    assert!(fragments.iter().any(
        |part| part.url.as_deref() == Some("https://example.org/@alice")
            && part.text.contains("@alice")
    ));
    assert!(fragments
        .iter()
        .any(|part| part.url.is_none() && part.text.contains("unsafe")));
    assert!(html::plain("<p>A &amp; B</p><p>C</p>").contains("A & B"));
    for url in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "https://user:password@example.org",
    ] {
        assert!(!html::web_url(url));
    }
    assert_eq!(html::full_acct(&account()), "@alice@example.org");
}

#[test]
fn content_warning_and_sensitive_media_have_independent_reveal_controls() {
    let mut state = State::new(false);
    let mut post = post("s1", "Hidden body");
    post.spoiler_text = "CW".into();
    post.sensitive = true;
    assert!(!state.content_visible(&post));
    assert!(!state.media_visible(&post, false));
    state.warnings.insert(post.id.clone());
    assert!(state.content_visible(&post));
    assert!(!state.media_visible(&post, false));
    state.media.insert(post.id.clone());
    assert!(state.media_visible(&post, false));
    state.warnings.remove(&post.id);
    assert!(!state.media_visible(&post, false));
}

#[test]
fn filters_fail_closed_until_loaded_and_apply_context_expiry_and_unicode_boundaries() {
    let post = post("s1", "<p>A CAFÉ and a café.</p>");
    assert_eq!(
        filters::Filters::Loading.decision(&post, filters::Context::Home),
        filters::Decision::Loading
    );
    assert_eq!(
        filters::Filters::Failed.decision(&post, filters::Context::Home),
        filters::Decision::Failed
    );
    let filters = filters::Filters::ready(vec![filter("warn", "café", "home")]);
    assert!(matches!(
        filters.decision(&post, filters::Context::Home),
        filters::Decision::Warn(..)
    ));
    assert_eq!(
        filters.decision(&post, filters::Context::Public),
        filters::Decision::Allow
    );
    assert_eq!(
        filters.decision(
            &super::tests::post("s2", "caféteria"),
            filters::Context::Home
        ),
        filters::Decision::Allow
    );
    let expired: filters::Filter = serde_json::from_value(json!({"title":"Expired","filter_action":"hide","context":["home"],"expires_at":"2000-01-01T00:00:00Z","keywords":[{"keyword":"café","whole_word":false}]})).unwrap();
    assert_eq!(
        filters::Filters::ready(vec![expired]).decision(&post, filters::Context::Home),
        filters::Decision::Allow
    );
}

#[test]
fn filters_match_warnings_descriptions_and_boosts_with_hide_taking_precedence() {
    let mut post = post("s1", "Ordinary text");
    post.spoiler_text = "spoiler".into();
    let attachment: Attachment = serde_json::from_value(json!({"id":"m1","type":"image","url":"https://example.org/photo","description":"a rainbow"})).unwrap();
    post.media_attachments.push(attachment);
    assert!(matches!(
        filters::Filters::ready(vec![filter("blur", "rainbow", "home")])
            .decision(&post, filters::Context::Home),
        filters::Decision::Blur(_)
    ));
    let rules = filters::Filters::ready(vec![
        filter("warn", "spoiler", "home"),
        filter("hide", "rainbow", "home"),
    ]);
    assert_eq!(
        rules.decision(&post, filters::Context::Home),
        filters::Decision::Hide
    );
    let mut boost = super::tests::post("s2", "");
    boost.reblog = Some(Box::new(post));
    assert_eq!(
        rules.decision(&boost, filters::Context::Home),
        filters::Decision::Hide
    );
}

#[test]
fn status_id_filters_and_unknown_actions_are_respected() {
    let rule: filters::Filter = serde_json::from_value(json!({"title":"Specific post","filter_action":"future-action","context":["thread"],"expires_at":null,"statuses":[{"status_id":"s1"}]})).unwrap();
    let filters = filters::Filters::ready(vec![rule]);
    assert!(matches!(
        filters.decision(&post("s1", "unrelated"), filters::Context::Thread),
        filters::Decision::Warn(..)
    ));
    assert_eq!(
        filters.decision(&post("s2", "unrelated"), filters::Context::Thread),
        filters::Decision::Allow
    );
    let filters = filters::Filters::ready(vec![filter("hide", "C++", "home")]);
    assert_eq!(
        filters.decision(&post("s1", "I write C++ today"), filters::Context::Home),
        filters::Decision::Hide
    );
    assert_eq!(
        filters.decision(&post("s2", "I write C today"), filters::Context::Home),
        filters::Decision::Allow
    );
}

#[test]
fn poll_validation_rejects_duplicates_invalid_choices_and_closed_polls() {
    let single = poll(false);
    let mut multiple = poll(true);
    assert!(valid_choices(&single, &[0]));
    for choices in [vec![], vec![0, 1], vec![2], vec![0, 0]] {
        assert!(!valid_choices(&single, &choices));
    }
    assert!(valid_choices(&multiple, &[0, 1]));
    assert!(!valid_choices(&multiple, &[1, 1]));
    assert!(poll_open(&multiple));
    multiple.expires_at = Some(chrono::Utc::now() - chrono::Duration::seconds(1));
    assert!(!poll_open(&multiple));
    multiple.expires_at = None;
    multiple.expired = true;
    assert!(!poll_open(&multiple));
}

#[test]
fn profile_pagination_deduplicates_posts_and_stops_at_the_end() {
    let mut profile = profile::Profile::new(1, account());
    let page: Vec<_> = (0..20)
        .map(|index| post(&index.to_string(), "text"))
        .collect();
    profile.append(&page[..1], &page);
    assert!(profile.more);
    assert_eq!(profile.max_id.as_deref(), Some("19"));
    profile.append(&[], &page[18..]);
    assert_eq!(profile.posts.len(), 20);
    assert_eq!(profile.pinned.len(), 1);
    assert!(!profile.more);
    profile.append(&[], &[]);
    assert!(!profile.more);
    assert_eq!(profile.max_id.as_deref(), Some("19"));
}

#[test]
fn clearing_account_cache_drops_reveal_overrides_and_poll_selections() {
    let mut cache = Cache::new();
    cache.reading.warnings.insert("s1".into());
    cache.reading.media.insert("s1".into());
    cache
        .reading
        .filter_reveals
        .insert(("s1".into(), filters::Context::Home));
    cache.reading.choices.insert("p1".into(), vec![0]);
    cache.clear();
    assert!(
        cache.reading.warnings.is_empty()
            && cache.reading.media.is_empty()
            && cache.reading.filter_reveals.is_empty()
            && cache.reading.choices.is_empty()
    );
}

#[test]
fn warning_and_blur_filters_keep_separate_media_protection() {
    let post = post("s1", "spoiler rainbow");
    let filters = filters::Filters::ready(vec![
        filter("warn", "spoiler", "home"),
        filter("blur", "rainbow", "home"),
    ]);
    let filters::Decision::Warn(warnings, media) = filters.decision(&post, filters::Context::Home)
    else {
        panic!("expected warning and media blur")
    };
    assert_eq!(warnings.len(), 1);
    assert_eq!(media.len(), 1);
    assert!(!State::new(false).media_visible(&post, !media.is_empty()));
}

#[test]
fn report_payload_preserves_target_and_only_forwards_remote_accounts() {
    let mut report = report::State::new(1, account(), Some("s1".into()));
    report.comment = "説明".repeat(500);
    assert!(report.can_submit());
    report.forward = true;
    let options = report.options();
    assert_eq!(options.status_ids, Some(vec!["s1".into()]));
    assert_eq!(options.forward, Some(false));
    report.account.acct = "alice@remote.example".into();
    assert_eq!(report.options().forward, Some(true));
    report.comment.push('字');
    assert!(!report.can_submit());
    report.comment.clear();
    report.sending = true;
    assert!(!report.can_submit());
}
