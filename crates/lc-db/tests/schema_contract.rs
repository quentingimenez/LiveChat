//! Garde-fou : la migration embarquée et le contrat humain `docs/rust/schema-v3.sql`
//! doivent rester strictement identiques.
//!
//! Toute évolution de schéma se fait dans `crates/lc-db/migrations/0001_init.sql`
//! (source unique, consommée par `lc-db` et `xtask`), puis la copie
//! `docs/rust/schema-v3.sql` est régénérée. Ce test échoue dès qu'elles divergent.

#[test]
fn migration_embedded_equals_schema_contract() {
    let contract = include_str!("../../../docs/rust/schema-v3.sql");
    assert_eq!(
        lc_db::MIGRATION_SQL,
        contract,
        "crates/lc-db/migrations/0001_init.sql et docs/rust/schema-v3.sql ont divergé : \
         mettre à jour la migration puis recopier le contrat dans docs/rust/"
    );
}
