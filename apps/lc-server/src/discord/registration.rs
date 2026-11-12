//! Enregistrement des commandes slash Discord.
//!
//! Fidélité legacy : `server/src/loaders/DiscordLoader.ts` enregistrait les
//! commandes **globalement** (`Routes.applicationCommands(DISCORD_CLIENT_ID)`)
//! à partir du catalogue, une seule fois au démarrage, dans l'ordre historique
//! (les commandes « cachées » `hidesend`/`hidetalk` n'étant incluses que si
//! `HIDE_COMMANDS_DISABLED !== "true"`).
//!
//! Le payload est produit par le crate [`lc_discord`] (feature `serenity`) :
//! c'est le même JSON que celui envoyé par le legacy, en français ou en anglais
//! selon `I18N` (une seule langue, pas de localisation par locale Discord).

use lc_discord::serenity_glue::{command_payload, command_payloads};
use lc_discord::{CommandSpec, Lang};
use serenity::all::{Command, Http};

use super::DiscordError;

/// Payloads JSON des commandes à enregistrer, dans l'ordre legacy.
pub fn registration_payloads(lang: Lang, hide_commands_disabled: bool) -> Vec<serde_json::Value> {
    command_payloads(lang, hide_commands_disabled)
}

/// Payload JSON d'une commande précise (exposé pour la doc et les tests).
pub fn payload_for(spec: &CommandSpec, lang: Lang) -> serde_json::Value {
    command_payload(spec, lang)
}

/// Enregistre les commandes globales du bot.
///
/// `POST/PUT application commands` n'a besoin que du token : serenity déduit
/// l'`application_id` du token (`Http::create_global_commands`).
pub async fn register_global_commands(
    http: &Http,
    payloads: &[serde_json::Value],
) -> Result<Vec<Command>, DiscordError> {
    // `Http::create_global_commands` exige une map `Serialize + Sized` : on
    // matérialise la tranche en `Vec` (15 commandes au maximum).
    let body = payloads.to_vec();
    http.create_global_commands(&body)
        .await
        .map_err(|error| DiscordError::Registration(Box::new(error)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::registration_payloads;
    use lc_discord::Lang;

    #[test]
    fn payloads_fr_ordre_legacy_et_noms() {
        let payloads = registration_payloads(Lang::Fr, false);
        let names: Vec<&str> = payloads
            .iter()
            .filter_map(|payload| payload.get("name").and_then(|name| name.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![
                "dispo",
                "msg",
                "dire",
                "pair-code",
                "help",
                "info",
                "config-defaut",
                "config-displayfull",
                "config-max",
                "overlays",
                "stop",
                "meme-add",
                "lier",
                "cmsg",
                "cdire",
            ]
        );
    }

    #[test]
    fn payloads_masquent_les_commandes_cachees_quand_demande() {
        let payloads = registration_payloads(Lang::Fr, true);
        let names: Vec<&str> = payloads
            .iter()
            .filter_map(|payload| payload.get("name").and_then(|name| name.as_str()))
            .collect();
        assert_eq!(names.len(), 13);
        assert!(!names.contains(&"cmsg"));
        assert!(!names.contains(&"cdire"));
    }

    #[test]
    fn payload_contient_description_et_options_types() {
        let payloads = registration_payloads(Lang::Fr, false);
        let meme_add = payloads
            .iter()
            .find(|payload| payload.get("name").and_then(|name| name.as_str()) == Some("meme-add"))
            .unwrap();
        assert!(meme_add
            .get("description")
            .and_then(|value| value.as_str())
            .is_some_and(|text| !text.is_empty()));
        let options = meme_add
            .get("options")
            .and_then(|value| value.as_array())
            .unwrap();
        assert_eq!(options.len(), 4);
        let types: Vec<u64> = options
            .iter()
            .filter_map(|option| option.get("type").and_then(|value| value.as_u64()))
            .collect();
        assert_eq!(types, vec![3, 11, 3, 5]);
    }
}
