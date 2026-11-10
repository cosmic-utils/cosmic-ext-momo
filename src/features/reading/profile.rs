use super::filters;
use crate::{
    app,
    cache::Cache,
    client::Client,
    features::{
        accounts,
        status::{self, StatusOptions},
    },
    fl,
};
use cosmic::{widget, Element};
use megalodon::{
    entities::{Account, Status},
    megalodon::GetAccountStatusesInputOptions,
};

pub struct Profile {
    pub request: u64,
    pub account: Account,
    pub pinned: Vec<String>,
    pub posts: Vec<String>,
    pub max_id: Option<String>,
    pub loading: bool,
    pub more: bool,
    pub error: Option<String>,
}
impl Profile {
    pub fn new(request: u64, account: Account) -> Self {
        Self {
            request,
            account,
            pinned: vec![],
            posts: vec![],
            max_id: None,
            loading: true,
            more: true,
            error: None,
        }
    }
    pub fn append(&mut self, pinned: &[Status], posts: &[Status]) {
        for status in pinned {
            if !self.pinned.contains(&status.id) {
                self.pinned.push(status.id.clone());
            }
        }
        self.more = posts.len() >= 20
            && posts
                .last()
                .is_some_and(|post| Some(&post.id) != self.max_id.as_ref());
        if let Some(last) = posts.last() {
            self.max_id = Some(last.id.clone());
        }
        for status in posts {
            if !self.posts.contains(&status.id) {
                self.posts.push(status.id.clone());
            }
        }
        let excess = self
            .posts
            .len()
            .saturating_sub(crate::cache::MAX_RETAINED_ITEMS);
        self.posts.drain(..excess);
        self.loading = false;
        self.error = None;
    }
    pub fn view<'a>(&'a self, cache: &'a Cache) -> Element<'a, app::Message> {
        let spacing = cosmic::theme::active().cosmic().spacing;
        let mut items =
            widget::column![accounts::account(&self.account, cache).map(app::Message::Account)]
                .spacing(spacing.space_s);
        if let Some(notice) = super::filter_notice(cache) {
            return items.push(notice.map(app::Message::Status)).into();
        }
        for (pinned, ids, label) in [
            (true, &self.pinned, fl!("reading-pinned")),
            (false, &self.posts, fl!("reading-posts")),
        ] {
            if pinned && ids.is_empty() {
                continue;
            }
            items = items.push(widget::text::heading(label));
            for id in ids {
                if !pinned && self.pinned.contains(id) {
                    continue;
                }
                if let Some(post) = cache.statuses.get(id) {
                    if cache
                        .reading
                        .filters
                        .decision(post, filters::Context::Account)
                        == filters::Decision::Hide
                    {
                        continue;
                    }
                    items = items.push(
                        widget::container(
                            status::status(
                                post,
                                StatusOptions::all().context(filters::Context::Account),
                                cache,
                            )
                            .map(app::Message::Status),
                        )
                        .width(cosmic::iced::Length::Fill)
                        .class(cosmic::style::Container::Card),
                    );
                }
            }
        }
        if self.loading {
            items = items.push(widget::indeterminate_circular().size(24.));
        } else if let Some(error) = &self.error {
            items = items.push(widget::text(error)).push(
                widget::button::standard(fl!("reading-retry")).on_press(app::Message::ProfileMore),
            );
        } else if self.more {
            items = items.push(
                widget::button::standard(fl!("reading-load-more"))
                    .on_press(app::Message::ProfileMore),
            );
        } else if self.posts.is_empty() && self.pinned.is_empty() {
            items = items.push(widget::text(fl!("reading-no-posts")));
        }
        items.into()
    }
}

pub async fn fetch(
    client: Client,
    id: String,
    max_id: Option<String>,
) -> Result<(Vec<Status>, Vec<Status>), String> {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let pinned = if max_id.is_none() {
            client
                .get_account_statuses(
                    id.clone(),
                    Some(&GetAccountStatusesInputOptions {
                        pinned: Some(true),
                        ..Default::default()
                    }),
                )
                .await
                .map_err(|_| fl!("reading-profile-error"))?
                .json
        } else {
            vec![]
        };
        let posts = client
            .get_account_statuses(
                id,
                Some(&GetAccountStatusesInputOptions {
                    max_id,
                    limit: Some(20),
                    ..Default::default()
                }),
            )
            .await
            .map_err(|_| fl!("reading-profile-error"))?
            .json;
        Ok((pinned, posts))
    })
    .await
    .unwrap_or_else(|_| Err(fl!("reading-profile-error")))
}
