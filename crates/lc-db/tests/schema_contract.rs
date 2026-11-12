//! Garde-fou : les migrations embarquées et le contrat humain
//! `docs/rust/schema-v3.sql` doivent rester strictement identiques.
//!
//! Toute évolution de schéma se fait dans `crates/lc-db/migrations/` (source
//! unique, consommée par `lc-db` et `xtask`), puis la copie
//! `docs/rust/schema-v3.sql` est régénérée par concaténation des migrations dans
//! l'ordre des versions. Ce test échoue dès qu'elles divergent.

#[test]
fn migrations_embedded_equal_schema_contract() {
    let migrations = concat!(
        include_str!("../migrations/0001_init.sql"),
        include_str!("../migrations/0002_playback_single_playing.sql"),
        include_str!("../migrations/0003_guild_members.sql"),
        include_str!("../migrations/0004_discord_link_codes.sql"),
    );
    let contract = include_str!("../../../docs/rust/schema-v3.sql");
    assert_eq!(
        migrations, contract,
        "la concaténation de crates/lc-db/migrations/ et docs/rust/schema-v3.sql \
         a divergé : mettre à jour les migrations puis recopier le contrat dans docs/rust/"
    );
}
