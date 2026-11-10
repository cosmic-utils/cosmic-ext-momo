use megalodon::entities::Status;

use crate::client::{within_timeout, Client};

pub async fn set(
    client: &Client,
    id: String,
    was_favorited: bool,
    previous_count: Option<u32>,
) -> Result<Status, String> {
    let response = within_timeout(
        async {
            if was_favorited {
                client.unfavourite_status(id).await
            } else {
                client.favourite_status(id).await
            }
        },
        crate::fl!("action-error"),
    )
    .await?;
    let mut status = response.json;
    // Some servers confirm the new state before updating the aggregate count.
    // Only compensate for an unchanged count on a confirmed state transition;
    // preserve counts that the server has already updated.
    if status.favourited == Some(!was_favorited) && previous_count == Some(status.favourites_count)
    {
        status.favourites_count = if was_favorited {
            status.favourites_count.saturating_sub(1)
        } else {
            status.favourites_count.saturating_add(1)
        };
    }
    Ok(status)
}
