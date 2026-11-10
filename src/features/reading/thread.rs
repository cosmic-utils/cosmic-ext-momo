use super::filters;
use crate::{
    app,
    cache::Cache,
    client::Client,
    features::status::{self, StatusOptions},
    fl,
};
use cosmic::{iced::Length, widget, Element};
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
        self.view_content(cache, false)
    }

    pub fn media_view<'a>(&'a self, cache: &'a Cache) -> Element<'a, app::Message> {
        self.view_content(cache, true).map(|message| match message {
            app::Message::Status(message) => app::Message::MediaStatus(Box::new(message)),
            app::Message::RetryReader => app::Message::RetryMediaThread,
            message => message,
        })
    }

    /// Group each reply beneath its parent while preserving sibling order from the server.
    /// Missing parents and malformed cycles still appear once, as separate branches.
    pub(super) fn reply_tree<'a>(&'a self, cache: &Cache) -> Vec<(&'a str, usize)> {
        use std::collections::{HashMap, HashSet};
        let ids: HashSet<&str> = self.descendants.iter().map(String::as_str).collect();
        let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
        for id in &self.descendants {
            if id == &self.root {
                continue;
            }
            let parent = cache
                .statuses
                .get(id)
                .and_then(|post| post.in_reply_to_id.as_deref())
                .filter(|parent| *parent == self.root || ids.contains(parent))
                .unwrap_or(&self.root);
            children.entry(parent).or_default().push(id);
        }
        let mut result = vec![];
        let mut seen = HashSet::new();
        for start in
            std::iter::once(self.root.as_str()).chain(self.descendants.iter().map(String::as_str))
        {
            let mut pending = vec![(start, usize::from(start != self.root))];
            while let Some((id, depth)) = pending.pop() {
                if !seen.insert(id) {
                    continue;
                }
                result.push((id, depth));
                if let Some(replies) = children.get(id) {
                    pending.extend(replies.iter().rev().map(|id| (*id, depth + 1)));
                }
            }
        }
        result
    }

    fn view_content<'a>(&'a self, cache: &'a Cache, media: bool) -> Element<'a, app::Message> {
        let spacing = cosmic::theme::active().cosmic().spacing;
        if let Some(notice) = super::filter_notice(cache) {
            return notice.map(app::Message::Status);
        }
        // Build branches from the leaves upward, so each parent's rail spans its replies.
        let mut branches: Vec<(usize, Element<'a, app::Message>)> = vec![];
        for (id, depth) in self.reply_tree(cache).into_iter().rev() {
            if let Some(post) = cache.statuses.get(id) {
                if cache
                    .reading
                    .filters
                    .decision(post, filters::Context::Thread)
                    == filters::Decision::Hide
                {
                    continue;
                }
                let post = status::status(
                    post,
                    StatusOptions::new(!media, true, true, id != &self.root)
                        .context(filters::Context::Thread),
                    cache,
                )
                .map(app::Message::Status);
                let post: Element<'a, app::Message> = if media {
                    post
                } else {
                    widget::container(post)
                        .width(Length::Fill)
                        .class(cosmic::style::Container::Card)
                        .into()
                };
                let mut replies = widget::column![].spacing(spacing.space_s);
                let mut has_replies = false;
                while branches
                    .last()
                    .is_some_and(|(child_depth, _)| *child_depth > depth)
                {
                    let (_, child) = branches.pop().unwrap();
                    has_replies = true;
                    // The elbow meets the reply near its author row.
                    let child = if depth < if media { 3 } else { 5 } {
                        widget::row![
                            widget::container(
                                widget::divider::horizontal::light()
                                    .class(cosmic::theme::Rule::custom(connector_style))
                            )
                            .padding([28, 0, 0, 0])
                            .width(16),
                            child,
                        ]
                        .into()
                    } else {
                        child
                    };
                    replies = replies.push(child);
                }
                let mut branch = widget::column![post]
                    .spacing(spacing.space_s)
                    .width(Length::Fill);
                if has_replies {
                    if depth == 0 {
                        branch = branch.push(widget::id_container(
                            widget::text::heading(fl!("reading-replies")),
                            widget::Id::new("thread-replies"),
                        ));
                    }
                    let toggle = app::Message::ToggleReplies(id.to_owned());
                    if cache.reading.collapsed_replies.contains(id) {
                        branch = branch.push(
                            widget::button::standard(fl!("reading-expand-replies"))
                                .on_press(toggle),
                        );
                    } else if depth < if media { 3 } else { 5 } {
                        let rail = widget::button::custom(
                            widget::divider::vertical::light()
                                .class(cosmic::theme::Rule::custom(connector_style))
                                .height(Length::Fill),
                        )
                        .padding(0)
                        .width(4)
                        .height(Length::Fill)
                        .class(cosmic::style::Button::Image)
                        .on_press(toggle);
                        #[cfg(target_os = "linux")]
                        let rail = rail.name(fl!("reading-collapse-replies"));
                        let rail = widget::container(rail).padding([0, 6]).height(Length::Fill);
                        // A scrollable offers unbounded height. Measure the replies first;
                        // a fill-height rule in a row would expand the entire branch.
                        branch = branch.push(cosmic::iced::widget::stack![
                            replies.width(Length::Fill),
                            rail,
                        ]);
                    } else {
                        branch = branch
                            .push(
                                widget::button::text(fl!("reading-collapse-replies"))
                                    .on_press(toggle),
                            )
                            .push(replies);
                    }
                }
                branches.push((depth, branch.into()));
            }
        }
        let mut items = widget::column![].spacing(spacing.space_s);
        for (_, branch) in branches.into_iter().rev() {
            items = items.push(branch);
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

fn connector_style(theme: &cosmic::Theme) -> cosmic::iced::widget::rule::Style {
    let mut style =
        cosmic::iced::widget::rule::Catalog::style(theme, &cosmic::theme::Rule::LightDivider);
    style.radius = 2.0.into();
    style
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
