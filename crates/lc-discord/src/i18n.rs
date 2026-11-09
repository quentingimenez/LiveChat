//! Textes fr/en du bot Discord.
//!
//! **Copie fidèle** des dictionnaires legacy
//! `server/src/services/i18n/{fr,en}.ts` pour toutes les clés utilisées par les
//! commandes de `lc-discord`. Les placeholders `{{nom}}` sont conservés tels
//! quels et interpolés par [`interpolate`] (équivalent de `rosetty.t`).
//!
//! Quelques chaînes sont volontairement **non localisées** dans le legacy et le
//! restent donc en anglais dans le catalogue : `help` (« List of Commands » /
//! « Commands : ») et `info` (« Bot Informations », lien GitHub). Voir
//! [`crate::catalog`].

use std::fmt;

/// Langue d'affichage du bot.
///
/// Le legacy choisit une langue unique au démarrage via `env.I18N`
/// (`rosetty`) : les **noms** des commandes enregistrées auprès de Discord
/// dépendent donc de cette langue (`/msg` en fr, `/send` en en).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lang {
    /// Français (défaut legacy de `I18N`).
    #[default]
    Fr,
    /// Anglais.
    En,
}

impl Lang {
    /// Code ISO utilisé par la configuration (`I18N=fr|en`).
    pub const fn code(self) -> &'static str {
        match self {
            Self::Fr => "fr",
            Self::En => "en",
        }
    }

    /// Décode `fr`/`en` (casse indifférente) ; toute autre valeur retombe sur
    /// le français, défaut historique de `env.I18N`.
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "en" => Self::En,
            _ => Self::Fr,
        }
    }
}

impl fmt::Display for Lang {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

/// Déclare la structure de textes et les deux variantes fr/en côte à côte.
macro_rules! strings {
    ($( $field:ident : ($fr:literal, $en:literal) ),+ $(,)?) => {
        /// Tous les textes du bot pour une langue donnée.
        ///
        /// Champs `snake_case` inchangés par rapport au legacy ; les
        /// placeholders `{{…}}` font partie de la chaîne.
        #[derive(Clone, Copy, Debug)]
        pub struct Strings {
            $(
                /// Texte localisé.
                pub $field: &'static str,
            )+
        }

        impl Strings {
            /// Toutes les variantes connues, indexées par [`Lang::index`].
            pub const ALL: [&'static Self; 2] = [&FR, &EN];
        }

        /// Dictionnaire français.
        pub const FR: Strings = Strings {
            $( $field: $fr, )+
        };

        /// Dictionnaire anglais.
        pub const EN: Strings = Strings {
            $( $field: $en, )+
        };
    };
}

strings! {
    command_error: (
        "Problème avec cette commande ! Veuillez vérifier les logs !",
        "Issue with this command! Please check logs!"
    ),
    success: ("Succès !", "Success!"),
    error: ("Erreur !", "Error!"),
    not_allowed: ("Action non autorisée !", "Not allowed!"),

    alive_name: ("dispo", "alive"),
    alive_description: ("Vérifiez si le bot est vivant", "Check if bot is alive"),
    alive_answer: ("{{username}}, Je suis en vie !", "{{username}}, I'm alive!"),

    pair_code_name: ("pair-code", "pair-code"),
    pair_code_description: (
        "Générer un code d’appairage à usage unique pour Overlay EXE ou LiveChat Extension",
        "Generate a one-time pairing code for Overlay EXE or LiveChat Extension"
    ),
    pair_code_option_mode: ("mode", "mode"),
    pair_code_option_mode_description: (
        "Mode d’appairage (normal ou invité lecture seule)",
        "Pairing mode (normal or read-only invite)"
    ),
    pair_code_choice_normal: ("normal", "normal"),
    pair_code_choice_invite: ("invité (overlay lecture seule)", "invite (overlay read-only)"),
    pair_code_answer_title: ("Code d’appairage Overlay", "Overlay pairing code"),
    pair_code_answer_description: (
        "API_URL :\n```{{apiUrl}}```\nCode d’appairage :\n```{{code}}```\nExpire dans {{expiresIn}} minute(s).",
        "API_URL:\n```{{apiUrl}}```\nPairing code:\n```{{code}}```\nExpires in {{expiresIn}} minute(s)."
    ),
    pair_code_answer_description_invite: (
        "API_URL :\n```{{apiUrl}}```\nCode d’appairage :\n```{{code}}```\nMode : invité lecture seule (Overlay EXE uniquement).\nCode à usage unique sans expiration.",
        "API_URL:\n```{{apiUrl}}```\nPairing code:\n```{{code}}```\nMode: invite read-only (Overlay EXE only).\nOne-time code without expiration."
    ),

    meme_add_name: ("meme-add", "meme-add"),
    meme_add_description: (
        "Ajouter un media a la meme board persistante",
        "Add media to the persistent meme board"
    ),
    meme_add_option_url: ("lien", "link"),
    meme_add_option_url_description: (
        "Lien du media a enregistrer dans la meme board",
        "Link to media to store in meme board"
    ),
    meme_add_option_media: ("media", "media"),
    meme_add_option_media_description: (
        "Media en piece jointe a enregistrer dans la meme board",
        "Attachment media to store in meme board"
    ),
    meme_add_option_title: ("titre", "title"),
    meme_add_option_title_description: (
        "Titre optionnel pour la meme board",
        "Optional display title for meme board"
    ),
    meme_add_option_force_refresh: ("refresh", "refresh"),
    meme_add_option_force_refresh_description: (
        "Ignorer le cache media et re-telecharger/re-transcoder la source",
        "Bypass media cache and re-download/re-transcode this source"
    ),
    meme_add_missing_media: (
        "Veuillez fournir au moins une source media (lien ou piece jointe).",
        "Please provide at least one media source (link or attachment)."
    ),
    meme_add_answer_created: ("Meme ajoute a la board.", "Meme added to board."),
    meme_add_answer_exists: (
        "Ce media est deja present dans la meme board.",
        "This media is already in the meme board."
    ),

    send_name: ("msg", "send"),
    send_description: ("Envoyer du contenu sur le stream", "Send content to stream"),
    send_option_url: ("lien", "link"),
    send_option_url_description: ("Lien du contenu sur le stream", "Link to the content to display"),
    send_option_text: ("texte", "text"),
    send_option_text_description: ("Texte à afficher", "Text to display"),
    send_option_media: ("média", "media"),
    send_option_media_description: ("Média à afficher", "Media to display"),
    send_option_force_refresh: ("refresh", "refresh"),
    send_option_force_refresh_description: (
        "Ignorer le cache média et re-télécharger/re-transcoder la source",
        "Bypass media cache and re-download/re-transcode this source"
    ),
    send_answer: (
        "Contenu reçu ! Il sera bientôt joué !",
        "Content received! It will be played soon!"
    ),
    send_missing_content: (
        "Veuillez fournir au moins un texte, un média ou un lien.",
        "Please provide at least text, media, or link."
    ),

    hide_send_name: ("cmsg", "hsend"),
    hide_send_description: (
        "Envoyer du contenu sur le stream (mais caché 😈)",
        "Send content to stream (hidden 😈)"
    ),
    hide_send_option_url: ("lien", "link"),
    hide_send_option_url_description: ("Lien du contenu sur le stream", "Link to the content to display"),
    hide_send_option_text: ("texte", "text"),
    hide_send_option_text_description: ("Texte à afficher", "Text to display"),
    hide_send_option_media: ("média", "media"),
    hide_send_option_media_description: ("Média à afficher", "Media to display"),
    hide_send_option_force_refresh: ("refresh", "refresh"),
    hide_send_option_force_refresh_description: (
        "Ignorer le cache média et re-télécharger/re-transcoder la source",
        "Bypass media cache and re-download/re-transcode this source"
    ),
    hide_send_answer: (
        "Contenu reçu ! Il sera bientôt joué !",
        "Content received! It will be played soon!"
    ),

    talk_name: ("dire", "talk"),
    talk_description: (
        "Demandez à un bot de dire quelque chose",
        "Ask a bot to say something"
    ),
    talk_option_text: ("texte", "text"),
    talk_option_text_description: ("Texte à afficher", "Text to display"),
    talk_option_voice: ("dire", "voice"),
    talk_option_voice_description: ("Texte à dire", "Text to say"),
    talk_answer: (
        "Contenu reçu ! Il sera bientôt joué !",
        "Content received! It will be played soon!"
    ),
    talk_voice_error: (
        "La génération vocale a échoué. Réessayez avec un texte plus court ou plus tard.",
        "Voice generation failed. Try a shorter text or retry later."
    ),

    hide_talk_name: ("cdire", "htalk"),
    hide_talk_description: (
        "Demandez à un bot de dire quelque chose (mais caché 😈)",
        "Ask a bot to say something (hidden 😈)"
    ),
    hide_talk_option_text: ("texte", "text"),
    hide_talk_option_text_description: ("Texte à afficher", "Text to display"),
    hide_talk_option_voice: ("dire", "voice"),
    hide_talk_option_voice_description: ("Texte à dire", "Text to say"),
    hide_talk_answer: (
        "Contenu reçu ! Il sera bientôt joué !",
        "Content received! It will be played soon!"
    ),

    set_default_time_name: ("config-defaut", "config-defaulttime"),
    set_default_time_description: (
        "Définir le temps par défaut pour l'affichage (Par défaut : 5 secondes) (En secondes)",
        "Set default time for display (Default: 5 seconds) (In seconds) | 0 will reset value to default"
    ),
    set_default_time_option_text: ("nombre", "number"),
    set_default_time_option_text_description: ("Nombre de secondes", "Number of seconds"),
    set_default_time_answer: ("Le temps par défaut est défini !", "Default time set!"),

    set_max_time_name: ("config-max", "config-maxtime"),
    set_max_time_description: (
        "Définir le temps maximal pour l'affichage (En secondes) | 0 remet la valeur par défaut",
        "Set max time for display (In seconds) | 0 will reset value to default"
    ),
    set_max_time_option_text: ("nombre", "number"),
    set_max_time_option_text_description: ("Nombre de secondes", "Number of seconds"),
    set_max_time_answer: ("Temps maximum défini !", "Max time set!"),

    set_display_full_name: ("config-displayfull", "config-displayfull"),
    set_display_full_description: (
        "Option legacy (conservée pour compatibilité)",
        "Legacy option (kept for compatibility)"
    ),
    set_display_full_option_text: ("value", "boolean"),
    set_display_full_option_text_description: ("Oui / Non", "True / False"),
    set_display_full_answer: ("Valeur définie !", "Display full set!"),

    overlays_name: ("overlays", "overlays"),
    overlays_description: (
        "Lister les overlays actuellement connectés",
        "List currently connected overlays"
    ),
    overlays_answer_title: ("Overlays connectés ({{count}})", "Connected overlays ({{count}})"),
    overlays_answer_empty: (
        "Aucun overlay n'est actuellement connecté sur ce serveur.",
        "No overlay is currently connected to this server."
    ),

    stop_name: ("stop", "stop"),
    stop_description: ("Interrompre la lecture en cours", "Stop current playback"),
    stop_answer: ("Lecture interrompue !", "Playback stopped"),

    media_error_unsupported_source: (
        "Ce lien/source n’est pas supporté par le bot.",
        "This link/source is not supported by the bot."
    ),
    media_error_private: (
        "Ce média est privé, restreint ou nécessite une authentification (cookies/compte) pour être récupéré.",
        "This media is private, restricted, or requires authentication (cookies/account) to access."
    ),
    media_error_drm: (
        "Ce média semble protégé par DRM et ne peut pas être téléchargé par le bot.",
        "This media seems DRM-protected and cannot be downloaded by the bot."
    ),
    media_error_not_found: (
        "Média introuvable (lien supprimé, expiré ou invalide).",
        "Media not found (link removed, expired, or invalid)."
    ),
    media_error_timeout: (
        "Le téléchargement a expiré (timeout). Réessayez dans quelques instants.",
        "The media download timed out. Please retry in a moment."
    ),
    media_error_too_large: (
        "Le média est trop volumineux (limite actuelle: {{maxSizeMb}} MB).",
        "The media is too large (current limit: {{maxSizeMb}} MB)."
    ),
    media_error_cache_storage_limit: (
        "Le cache média non persistant est plein (limite: {{maxCacheTotalMb}} MB). Réessayez dans un instant.",
        "The non-persistent media cache is full (limit: {{maxCacheTotalMb}} MB). Retry in a moment."
    ),
    media_error_board_storage_limit: (
        "Le stockage persistant de la mème board est plein (limite: {{maxBoardTotalMb}} MB). Supprimez des éléments avant un nouvel ajout.",
        "Persistent meme board storage is full (limit: {{maxBoardTotalMb}} MB). Remove older board items first."
    ),
    media_error_invalid_media: (
        "Le fichier téléchargé est invalide ou corrompu.",
        "The downloaded file is invalid or corrupted."
    ),
    media_error_transcode: (
        "La conversion du média a échoué (codec/format).",
        "Media conversion failed (codec/format issue)."
    ),
    media_error_download: (
        "Le média n’a pas pu être téléchargé depuis cette source.",
        "The media could not be downloaded from this source."
    ),
}

/// Renvoie le dictionnaire de `lang`.
pub fn strings(lang: Lang) -> &'static Strings {
    match lang {
        Lang::Fr => &FR,
        Lang::En => &EN,
    }
}

/// Remplace les placeholders `{{nom}}` d'un template.
///
/// Équivalent minimal de `rosetty.t(template, vars)` : les variables absentes
/// sont laissées telles quelles, ce qui rend les oublis visibles dans les tests
/// plutôt que silencieux.
pub fn interpolate(template: &str, vars: &[(&str, &str)]) -> String {
    let mut output = template.to_owned();
    for (name, value) in vars {
        let placeholder = format!("{{{{{name}}}}}");
        output = output.replace(&placeholder, value);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{interpolate, strings, Lang};

    #[test]
    fn variantes_fr_en_coherentes() {
        // Les deux dictionnaires existent et diffèrent sur les libellés
        // réellement localisés.
        assert_ne!(strings(Lang::Fr).send_name, strings(Lang::En).send_name);
        assert_eq!(
            strings(Lang::Fr).overlays_name,
            strings(Lang::En).overlays_name
        );
        assert_eq!(
            strings(Lang::Fr).pair_code_name,
            strings(Lang::En).pair_code_name
        );
        assert_ne!(
            super::FR.media_error_download,
            super::EN.media_error_download
        );
    }

    #[test]
    fn interpolation_placeholders() {
        let text = interpolate("{{username}}, I'm alive!", &[("username", "Jean")]);
        assert_eq!(text, "Jean, I'm alive!");

        let untouched = interpolate(
            "Expire dans {{expiresIn}} minute(s).",
            &[("code", "AB12CD")],
        );
        assert_eq!(untouched, "Expire dans {{expiresIn}} minute(s).");
    }

    #[test]
    fn parse_langue() {
        assert_eq!(Lang::parse("fr"), Lang::Fr);
        assert_eq!(Lang::parse("EN"), Lang::En);
        assert_eq!(Lang::parse("es"), Lang::Fr);
        assert_eq!(Lang::default(), Lang::Fr);
        assert_eq!(Lang::En.code(), "en");
    }
}
