use crate::{app, fl};
use cosmic::{widget, Element};
use megalodon::entities::Account;

#[derive(Debug, Clone)]
pub struct State {
    pub request: u64,
    pub account: Account,
    pub status_id: Option<String>,
    pub comment: String,
    pub forward: bool,
    pub sending: bool,
    pub error: Option<String>,
}
impl State {
    pub fn new(request: u64, account: Account, status_id: Option<String>) -> Self {
        Self {
            request,
            account,
            status_id,
            comment: String::new(),
            forward: false,
            sending: false,
            error: None,
        }
    }
    pub fn can_submit(&self) -> bool {
        !self.sending && self.comment.chars().count() <= 1000
    }
    pub fn options(&self) -> megalodon::megalodon::ReportInputOptions {
        megalodon::megalodon::ReportInputOptions {
            comment: Some(self.comment.trim().to_string()),
            status_ids: self.status_id.clone().map(|id| vec![id]),
            forward: Some(self.forward && self.account.acct.contains('@')),
            ..Default::default()
        }
    }
    pub fn view(&self) -> Element<'_, app::Message> {
        let mut controls = widget::column![
            widget::text(super::html::full_acct(&self.account)),
            widget::text(if self.status_id.is_some() {
                fl!("reading-report-post-description")
            } else {
                fl!("reading-report-account-description")
            }),
        ]
        .spacing(12);
        let input = widget::text_input(fl!("reading-report-reason"), &self.comment);
        controls = controls.push(if self.sending {
            input
        } else {
            input.on_input(app::Message::ReportComment)
        });
        controls = controls.push(widget::text::caption(format!(
            "{}/1000",
            self.comment.chars().count()
        )));
        if self.account.acct.contains('@') {
            let toggle = widget::toggler(self.forward)
                .label(fl!("reading-report-forward"))
                .spacing(8);
            controls = controls.push(if self.sending {
                toggle
            } else {
                toggle.on_toggle(app::Message::ReportForward)
            });
        }
        if let Some(error) = &self.error {
            controls = controls.push(widget::text(error));
        }
        if self.sending {
            controls = controls.push(widget::indeterminate_circular().size(24.));
        }
        widget::dialog()
            .title(fl!("reading-report"))
            .control(controls)
            .primary_action(
                widget::button::destructive(fl!("reading-report-submit"))
                    .on_press_maybe(self.can_submit().then_some(app::Message::SubmitReport)),
            )
            .secondary_action(widget::button::standard(fl!("cancel")).on_press_maybe(
                (!self.sending).then_some(app::Message::Dialog(app::DialogAction::Close)),
            ))
            .into()
    }
}
