//! Vidage du board média d'une guilde, partagé par l'overlay et la console.
//!
//! | Méthode | Chemin | Droit |
//! |---------|--------|-------|
//! | `POST` | `/overlay/admin/media-board/clear` | `ManageSettings` |
//!
//! La console du propriétaire appelle la même fonction
//! (`POST /admin/api/guilds/{guildId}/media-board/clear`) : les deux routes
//! arrêtent la lecture, suppriment les éléments du board et effacent les médias
//! devenus orphelins (fichier compris). Un média encore référencé par un job de
//! lecture ou un autre board est conservé.

use axum::response::{IntoResponse as _, Response};
use lc_domain::GuildId;
use lc_protocol::{BoardChange, BoardChangedPayload, ServerMessage};

use crate::app::AppState;
use crate::http::admin::hub::admin_hub;
use crate::http::admin::queries;
use crate::http::admin::remove_orphan_media;

use super::{db, db_failure, now};

/// Compteurs d'un vidage de board, communs aux deux routes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BoardClear {
    /// Jobs `PLAYING` repassés `DONE` par l'arrêt de lecture.
    pub(crate) released_playing_jobs: u64,
    /// Sockets ayant reçu l'arrêt (0 si le hub ne le compte pas).
    pub(crate) broadcast: usize,
    /// Éléments du board supprimés.
    pub(crate) media_board_items: u64,
    /// Médias devenus orphelins supprimés (ligne et fichier).
    pub(crate) orphan_media_assets: u64,
    /// Fichiers orphelins dont la suppression a échoué.
    pub(crate) orphan_media_file_delete_errors: u64,
}

/// Arrête la lecture de la guilde, vide son board et supprime ses médias
/// orphelins.
///
/// Les erreurs sont déjà des réponses HTTP prêtes (mêmes statuts et codes que
/// les autres routes overlay).
pub(crate) async fn clear_board(
    state: &AppState,
    guild_id: &GuildId,
) -> Result<BoardClear, Response> {
    let db = db(state)?;
    let hub = admin_hub(state);

    // Arrêt de la lecture avant la suppression : un job `PLAYING` ne doit pas
    // continuer à référencer un média que l'on s'apprête à effacer.
    let broadcast = hub.broadcast_stop(guild_id).await;
    let released_playing_jobs =
        queries::release_playing_jobs(db, guild_id.as_str(), lc_domain::datetime_to_ms(now(state)))
            .await
            .map_err(|db_error| db_failure("overlay_board_clear.release_jobs", &db_error))?;
    let has_row = db
        .guilds()
        .get(guild_id)
        .await
        .map_err(|db_error| db_failure("overlay_board_clear.guild", &db_error))?
        .is_some();
    if has_row {
        if let Err(db_error) = db.guilds().set_busy_until(guild_id, None).await {
            return Err(db_failure("overlay_board_clear.busy_until", &db_error));
        }
    }

    let candidate_media_asset_ids = queries::guild_board_media_asset_ids(db, guild_id.as_str())
        .await
        .map_err(|db_error| db_failure("overlay_board_clear.asset_ids", &db_error))?;
    let item_ids = queries::guild_board_item_ids(db, guild_id.as_str())
        .await
        .map_err(|db_error| db_failure("overlay_board_clear.item_ids", &db_error))?;
    let deleted_media_board_items =
        queries::delete_media_board_items_for_guild(db, guild_id.as_str())
            .await
            .map_err(|db_error| db_failure("overlay_board_clear.delete_items", &db_error))?;
    if deleted_media_board_items > 0 {
        for item_id in item_ids {
            hub.broadcast_to_guild(
                guild_id,
                ServerMessage::BoardChanged(BoardChangedPayload {
                    change: BoardChange::Deleted,
                    item_id,
                }),
            );
        }
    }
    let (deleted_orphan_media_assets, orphan_media_file_delete_errors) =
        remove_orphan_media(state, db, &candidate_media_asset_ids)
            .await
            .map_err(|error| error.into_response())?;

    tracing::info!(
        guild_id = guild_id.as_str(),
        media_board_items = deleted_media_board_items,
        orphan_media_assets = deleted_orphan_media_assets,
        orphan_media_file_delete_errors,
        released_playing_jobs,
        broadcast,
        "board média vidé"
    );

    Ok(BoardClear {
        released_playing_jobs,
        broadcast,
        media_board_items: deleted_media_board_items,
        orphan_media_assets: deleted_orphan_media_assets,
        orphan_media_file_delete_errors,
    })
}
