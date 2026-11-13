//! Tests du repository `pairing_codes` : consommation atomique, expiration,
//! révocation et purge.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use chrono::Duration;
use lc_db::DbError;
use lc_domain::{DomainError, PairingCode as PairingCodeValue};

#[tokio::test]
async fn crud_normalisation_et_doublon() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.pairing_codes();
    let now = common::now();
    let guild = common::guild("800000000000000000");

    let code = common::sample_pairing("ab12cd", &guild, now);
    assert_eq!(code.code.as_str(), "AB12CD", "le code est normalisé");
    repo.create(&code).await.unwrap();

    // Lecture insensible à la casse et aux espaces (parité routes legacy).
    let looked_up = repo
        .get(&PairingCodeValue::parse("  ab12cd ").unwrap())
        .await
        .unwrap()
        .expect("code présent");
    assert_eq!(looked_up, code);
    assert!(repo
        .get(&PairingCodeValue::parse("ZZZZZZ").unwrap())
        .await
        .unwrap()
        .is_none());

    assert_eq!(repo.list().await.unwrap().len(), 1);
    assert_eq!(repo.list_by_guild(&guild).await.unwrap().len(), 1);

    let error = repo
        .create(&common::sample_pairing("AB12CD", &guild, now))
        .await
        .unwrap_err();
    match error {
        DbError::Domain(DomainError::Conflict(message)) => {
            assert!(message.contains("déjà"), "{message}");
        }
        other => panic!("erreur inattendue : {other}"),
    }
}

#[tokio::test]
async fn consume_met_a_jour_used_at_une_seule_fois() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.pairing_codes();
    let now = common::now();
    let guild = common::guild("801111111111111111");

    let code = common::sample_pairing("USED01", &guild, now);
    repo.create(&code).await.unwrap();

    let consumed = repo
        .consume(&code.code, now + Duration::seconds(1))
        .await
        .unwrap()
        .expect("première consommation");
    assert_eq!(consumed.used_at, Some(now + Duration::seconds(1)));
    assert!(repo
        .consume(&code.code, now + Duration::seconds(2))
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        repo.get(&code.code).await.unwrap().unwrap().used_at,
        Some(now + Duration::seconds(1))
    );
    assert!(repo
        .consume(&PairingCodeValue::parse("NOPE99").unwrap(), now)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn consume_concurrent_un_seul_gagnant() {
    let (db, _dir) = common::temp_db().await;
    let now = common::now();
    let guild = common::guild("802222222222222222");
    let code = common::sample_pairing("RACE99", &guild, now);
    db.pairing_codes().create(&code).await.unwrap();

    let first = db.clone();
    let second = db.clone();
    let code_first = code.code.clone();
    let code_second = code.code.clone();
    let first_repo = first.pairing_codes();
    let second_repo = second.pairing_codes();
    let deadline = now + Duration::seconds(1);
    let (first, second) = tokio::join!(
        first_repo.consume(&code_first, deadline),
        second_repo.consume(&code_second, deadline)
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(
        first.iter().chain(second.iter()).count(),
        1,
        "un seul consommateur doit gagner"
    );
    assert!(db
        .pairing_codes()
        .get(&code.code)
        .await
        .unwrap()
        .unwrap()
        .used_at
        .is_some());
}

#[tokio::test]
async fn expiration_et_codes_permanents() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.pairing_codes();
    let now = common::now();
    let guild = common::guild("803333333333333333");

    // Code expiré : jamais consommé.
    let mut expired = common::sample_pairing("EXP999", &guild, now);
    expired.expires_at = Some(now - Duration::milliseconds(1));
    repo.create(&expired).await.unwrap();
    assert!(repo.consume(&expired.code, now).await.unwrap().is_none());

    // Code permanent (invité, `expires_at = None`) : consommable longtemps après.
    let mut permanent = common::sample_pairing("PERM22", &guild, now);
    permanent.expires_at = None;
    repo.create(&permanent).await.unwrap();
    assert!(repo
        .consume(&permanent.code, now + Duration::days(365))
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn revocation_et_purge_des_inutilisables() {
    let (db, _dir) = common::temp_db().await;
    let repo = db.pairing_codes();
    let now = common::now();
    let guild = common::guild("804444444444444444");

    let revoked = common::sample_pairing("REV111", &guild, now);
    repo.create(&revoked).await.unwrap();
    assert!(repo
        .revoke(&revoked.code, now + Duration::seconds(1))
        .await
        .unwrap());
    assert!(repo
        .consume(&revoked.code, now + Duration::seconds(2))
        .await
        .unwrap()
        .is_none());
    assert!(!repo
        .revoke(&revoked.code, now + Duration::seconds(3))
        .await
        .unwrap());

    let used = common::sample_pairing("USED22", &guild, now);
    repo.create(&used).await.unwrap();
    assert!(repo
        .consume(&used.code, now + Duration::seconds(1))
        .await
        .unwrap()
        .is_some());

    let mut expires_soon = common::sample_pairing("SOON33", &guild, now);
    expires_soon.expires_at = Some(now + Duration::seconds(1));
    repo.create(&expires_soon).await.unwrap();

    let valid = common::sample_pairing("KEEP44", &guild, now);
    repo.create(&valid).await.unwrap();

    assert_eq!(
        repo.delete_expired(now + Duration::seconds(2))
            .await
            .unwrap(),
        3,
        "révoqué + consommé + expiré supprimés"
    );
    assert!(repo.get(&revoked.code).await.unwrap().is_none());
    assert!(repo.get(&used.code).await.unwrap().is_none());
    assert!(repo.get(&expires_soon.code).await.unwrap().is_none());
    assert!(repo.get(&valid.code).await.unwrap().is_some());
}
