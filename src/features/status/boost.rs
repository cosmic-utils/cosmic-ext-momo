use megalodon::entities::Status;

use crate::client::{within_timeout, Client};

pub async fn set(
    client: &Client,
    id: String,
    was_boosted: bool,
    previous_count: Option<u32>,
) -> Result<Status, String> {
    let response = within_timeout(
        async {
            if was_boosted {
                client.unreblog_status(id).await
            } else {
                client.reblog_status(id).await
            }
        },
        crate::fl!("action-error"),
    )
    .await?;
    let mut status = response.json;
    // Boost responses can wrap the original status. Its count and state are
    // what the UI displays, rather than the wrapper's own counters.
    let original = match &mut status.reblog {
        Some(original) => original.as_mut(),
        None => &mut status,
    };
    if original.reblogged == Some(!was_boosted) && previous_count == Some(original.reblogs_count) {
        original.reblogs_count = if was_boosted {
            original.reblogs_count.saturating_sub(1)
        } else {
            original.reblogs_count.saturating_add(1)
        };
    }
    Ok(status)
}
