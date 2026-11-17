use serenity::all::{
    ChannelId,
    CommandInteraction,
    EditChannel,
    EditInteractionResponse,
    GuildId,
    Http,
    HttpError,
    JsonErrorCode,
    PermissionOverwriteType,
};
use serenity::nonmax::NonMaxU16;
use sqlx::PgPool;

use crate::error::PermissionError;
use crate::{Result, TempVoiceError, VoiceChannelRow};

pub(super) async fn reset(
    http: &Http,
    interaction: &CommandInteraction,
    pool: &PgPool,
    guild_id: GuildId,
    channel_id: ChannelId,
    mut row: VoiceChannelRow,
) -> Result<()> {
    interaction.defer_ephemeral(http).await?;

    if !row.is_owner(interaction.user.id) {
        return Err(TempVoiceError::MissingPermissions(PermissionError::NotOwner));
    }

    row.reset();
    row.save(pool).await?;

    let channel = match channel_id.to_guild_channel(http, Some(guild_id)).await {
        Err(serenity::Error::Http(HttpError::UnsuccessfulRequest(resp)))
            if resp.error.code == JsonErrorCode::UnknownChannel =>
        {
            return Err(TempVoiceError::ChannelNotFound(channel_id));
        },
        r => r?,
    };

    let everyone_permissions = channel
        .permission_overwrites
        .iter()
        .find(|perm| {
            perm.kind == PermissionOverwriteType::Role(guild_id.everyone_role())
        })
        .ok_or(TempVoiceError::IneligibleChannel)?;

    channel_id
        .edit(
            http,
            EditChannel::new()
                .name(format!("{}'s Channel", interaction.user.display_name()))
                .user_limit(NonMaxU16::ZERO)
                .permissions(vec![everyone_permissions.clone()]),
        )
        .await?;

    interaction
        .edit_response(
            http,
            EditInteractionResponse::new().content("Reset channel."),
        )
        .await?;

    Ok(())
}
