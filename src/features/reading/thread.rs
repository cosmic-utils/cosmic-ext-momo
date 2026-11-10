use super::filters;
use crate::{
    app,
    cache::Cache,
    client::Client,
    features::status::{self, StatusOptions},
    fl,
};
use cosmic::{widget, Element};
use megalodon::entities::{Context, Status};

pub struct Thread {
    pub request: u64,
    pub root: String,
    pub ancestors: Vec<String>,
    pub descendants: Vec<String>,
    pub loading: bool,
    pub error: Option<String>,
}
impl Thread {
    pub fn new(request: u64, root: String) -> Self {
        Self {
            request,
            root,
            ancestors: vec![],
            descendants: vec![],
            loading: true,
            error: None,
        }
    }
    pub fn view<'a>(&'a self, cache: &'a Cache) -> Element<'a, app::Message> {
        let spacing = cosmic::theme::active().cosmic().spacing;
        if let Some(notice) = super::filter_notice(cache) {
            return notice.map(app::Message::Status);
        }
        let mut items = widget::column![].spacing(spacing.space_s);
        for (ids, label) in [
            (self.ancestors.as_slice(), Some(fl!("reading-ancestors"))),
            (std::slice::from_ref(&self.root), None),
            (self.descendants.as_slice(), Some(fl!("reading-replies"))),
        ] {
            if ids.is_empty() {
                continue;
            }
            if let Some(label) = label {
                items = items.push(widget::text::heading(label));
            }
            for id in ids {
                if let Some(post) = cache.statuses.get(id) {
                    if cache
                        .reading
                        .filters
                        .decision(post, filters::Context::Thread)
                        == filters::Decision::Hide
                    {
                        continue;
                    }
                    items = items.push(
                        status::status(
                            post,
                            StatusOptions::new(true, true, true, id != &self.root)
                                .context(filters::Context::Thread),
                            cache,
                        )
                        .map(app::Message::Status),
                    );
                }
            }
        }
        if self.loading {
            items = items.push(widget::indeterminate_circular().size(24.));
        }
        if let Some(error) = &self.error {
            items = items.push(widget::text(error)).push(
                widget::button::standard(fl!("reading-retry")).on_press(app::Message::RetryReader),
            );
        }
        items.into()
    }
}

pub async fn fetch(client: Client, id: String) -> Result<(Status, Context), String> {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let post = client
            .get_status(id.clone())
            .await
            .map_err(|_| fl!("reading-thread-error"))?
            .json;
        let context = client
            .get_status_context(id, None)
            .await
            .map_err(|_| fl!("reading-thread-error"))?
            .json;
        Ok((post, context))
    })
    .await
    .unwrap_or_else(|_| Err(fl!("reading-thread-error")))
}
