//! Tests des repositories `overlay_clients` et `ingest_clients`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use chrono::Duration;
use lc_db::DbError;
use lc_domain::{DomainError, OverlaySessionMode};

#[tokio::test]
async fn overlay_crud_et_revocation() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.overlay_clients();
    let now = common::now();
    let guild = common::guild("700000000000000000");
    let other_guild = common::guild("701111111111111111");

    let client = common::sample_overlay(&guild, "hash-overlay-1", now);
    repo.create(&client).await.unwrap();

    assert_eq!(repo.get(client.id).await.unwrap().unwrap(), client);
    let authenticated = repo
        .get_by_token_hash("hash-overlay-1")
        .await
        .unwrap()
        .expect("token actif");
    assert_eq!(authenticated.id, client.id);
    assert!(repo
        .get_by_token_hash("hash-inconnu")
        .await
        .unwrap()
        .is_none());

    assert_eq!(repo.list_by_guild(&guild).await.unwrap().len(), 1);
    assert!(repo.list_by_guild(&other_guild).await.unwrap().is_empty());

    let seen_at = now + Duration::seconds(3);
    assert!(repo.touch_last_seen(client.id, seen_at).await.unwrap());
    assert_eq!(
        repo.get(client.id).await.unwrap().unwrap().last_seen_at,
        Some(seen_at)
    );

    assert!(repo
        .update_mode(client.id, OverlaySessionMode::InviteReadOnly)
        .await
        .unwrap());
    assert_eq!(
        repo.get(client.id).await.unwrap().unwrap().session_mode,
        OverlaySessionMode::InviteReadOnly
    );

    let revoked_at = now + Duration::seconds(4);
    assert!(repo.revoke(client.id, revoked_at).await.unwrap());
    let revoked = repo.get(client.id).await.unwrap().unwrap();
    assert_eq!(revoked.revoked_at, Some(revoked_at));
    assert!(
        repo.get_by_token_hash("hash-overlay-1")
            .await
            .unwrap()
            .is_none(),
        "un overlay révoqué ne s'authentifie plus"
    );
    // Révocation et changement de mode idempotents : plus d'effet.
    assert!(!repo
        .revoke(client.id, now + Duration::seconds(5))
        .await
        .unwrap());
    assert!(!repo
        .update_mode(client.id, OverlaySessionMode::Normal)
        .await
        .unwrap());

    // Le hash reste UNIQUE même pour un client révoqué.
    let error = repo
        .create(&common::sample_overlay(&guild, "hash-overlay-1", now))
        .await
        .unwrap_err();
    match error {
        DbError::Domain(DomainError::Conflict(message)) => {
            assert!(message.contains("hash de token"), "{message}");
        }
        other => panic!("erreur inattendue : {other}"),
    }
}

#[tokio::test]
async fn ingest_crud_et_revocation() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.ingest_clients();
    let now = common::now();
    let guild = common::guild("702222222222222222");
    let other_guild = common::guild("703333333333333333");

    let client = common::sample_ingest(&guild, "hash-ingest-1", now);
    repo.create(&client).await.unwrap();

    assert_eq!(repo.get(client.id).await.unwrap().unwrap(), client);
    let authenticated = repo
        .get_by_token_hash("hash-ingest-1")
        .await
        .unwrap()
        .expect("token actif");
    assert_eq!(authenticated.id, client.id);
    assert!(repo
        .get_by_token_hash("hash-inconnu")
        .await
        .unwrap()
        .is_none());

    assert_eq!(repo.list().await.unwrap().len(), 1);
    assert_eq!(repo.list_by_guild(&guild).await.unwrap().len(), 1);
    assert!(repo.list_by_guild(&other_guild).await.unwrap().is_empty());

    let seen_at = now + Duration::seconds(7);
    assert!(repo.touch_last_seen(client.id, seen_at).await.unwrap());
    assert_eq!(
        repo.get(client.id).await.unwrap().unwrap().last_seen_at,
        Some(seen_at)
    );

    let revoked_at = now + Duration::seconds(8);
    assert!(repo.revoke(client.id, revoked_at).await.unwrap());
    assert_eq!(
        repo.get(client.id).await.unwrap().unwrap().revoked_at,
        Some(revoked_at)
    );
    assert!(repo
        .get_by_token_hash("hash-ingest-1")
        .await
        .unwrap()
        .is_none());
    assert!(!repo.revoke(client.id, now).await.unwrap());

    let error = repo
        .create(&common::sample_ingest(&guild, "hash-ingest-1", now))
        .await
        .unwrap_err();
    assert!(
        matches!(error, DbError::Domain(DomainError::Conflict(_))),
        "hash de token dupliqué : {error}"
    );
}
