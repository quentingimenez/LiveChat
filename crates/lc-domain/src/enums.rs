//! Enums métier et leurs représentations.
//!
//! Deux représentations à ne pas confondre :
//! - **base** (`as_db_str` / `from_db_str`) : `TEXT` MAJUSCULE, contraint par
//!   les `CHECK` du schéma v3 (`IMAGE`, `INVITE_READ_ONLY`…), identique au
//!   legacy Prisma (les enums y étaient des `String` uppercase) ;
//! - **JSON** (derive serde, `snake_case`) : `image`, `invite_read_only`… pour
//!   les payloads de reprise v2 (`sessionMode` de `hello`, cf.
//!   `docs/rust/protocol-v2.md` §5.1).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// Déclare un enum « texte » avec conversion base + serde `snake_case`.
macro_rules! text_enum {
    (
        $(#[$meta:meta])*
        $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident => ($db:literal, $wire:literal)
            ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        ///
        /// JSON : `snake_case` (voyelles de transport) ; base : `TEXT`
        /// MAJUSCULE (contraintes `CHECK` du schéma v3).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $(
                $(#[$variant_meta])*
                $variant,
            )+
        }

        impl $name {
            /// Toutes les variantes, dans l'ordre déclaré.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// Représentation stockée en base (`TEXT` MAJUSCULE).
            pub const fn as_db_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $db),+
                }
            }

            /// Représentation JSON v2 (`snake_case`).
            pub const fn as_wire_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),+
                }
            }

            /// Décode la représentation base ; erreur de validation sinon.
            pub fn from_db_str(raw: &str) -> Result<Self, DomainError> {
                match raw {
                    $($db => Ok(Self::$variant),)+
                    other => Err(DomainError::validation(format!(
                        concat!(stringify!($name), " inconnu en base : {}"),
                        other
                    ))),
                }
            }

            /// Décode la représentation JSON v2 ; erreur de validation sinon.
            pub fn from_wire_str(raw: &str) -> Result<Self, DomainError> {
                match raw {
                    $($wire => Ok(Self::$variant),)+
                    other => Err(DomainError::validation(format!(
                        concat!(stringify!($name), " inconnu sur le fil : {}"),
                        other
                    ))),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_db_str())
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(raw: &str) -> Result<Self, Self::Err> {
                Self::from_db_str(raw)
            }
        }
    };
}

text_enum! {
    /// Famille d'un média ingéré (`media_assets.kind`).
    MediaKind {
        /// Image fixe (GIF animé inclus côté legacy).
        Image => ("IMAGE", "image"),
        /// Piste audio.
        Audio => ("AUDIO", "audio"),
        /// Vidéo.
        Video => ("VIDEO", "video"),
    }
}

text_enum! {
    /// État du pipeline média (`media_assets.status`).
    MediaStatus {
        /// Téléchargement / transcodage en cours.
        Processing => ("PROCESSING", "processing"),
        /// Prêt à être servi aux overlays.
        Ready => ("READY", "ready"),
        /// Échec définitif (détail dans `media_assets.error`).
        Failed => ("FAILED", "failed"),
    }
}

text_enum! {
    /// État d'un job de lecture (`playback_jobs.status`).
    JobStatus {
        /// En file, jamais démarré.
        Pending => ("PENDING", "pending"),
        /// En cours de lecture (au plus un par guilde).
        Playing => ("PLAYING", "playing"),
        /// Terminé normalement.
        Done => ("DONE", "done"),
        /// Terminé en erreur.
        Failed => ("FAILED", "failed"),
    }
}

text_enum! {
    /// Mode de session d'un overlay (`overlay_clients.session_mode`).
    OverlaySessionMode {
        /// Contrôle complet (arrêt, board média…).
        Normal => ("NORMAL", "normal"),
        /// Invité : lecture seule, les commandes d'arrêt sont refusées.
        InviteReadOnly => ("INVITE_READ_ONLY", "invite_read_only"),
    }
}

text_enum! {
    /// Mode d'un code d'appairage (`pairing_codes.mode`).
    PairingMode {
        /// Le client appairé peut déclencher/arrêter la lecture.
        Normal => ("NORMAL", "normal"),
        /// Le client appairé est en lecture seule.
        InviteReadOnly => ("INVITE_READ_ONLY", "invite_read_only"),
    }
}

text_enum! {
    /// Rôle d'un membre dans une guilde (`guild_members.role`).
    GuildRole {
        /// Gère la guilde : membres, invitations, réglages, modération.
        Admin => ("ADMIN", "admin"),
        /// Peut jouer, ajouter des médias et arrêter la lecture.
        Member => ("MEMBER", "member"),
        /// Reçoit les médias ; lecture seule.
        Guest => ("GUEST", "guest"),
    }
}

/// Action soumise à autorisation dans une guilde.
///
/// [`GuildRole::can`] est l'unique point de décision : les handlers ne
/// comparent jamais un rôle directement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Permission {
    /// Recevoir les médias diffusés sur l'overlay.
    ReceiveMedia,
    /// Consulter la file d'attente.
    ViewQueue,
    /// Consulter le board média (pour un invité : sous réserve du réglage de
    /// guilde, cf. [`GuildRole::can_view_board`]).
    ViewBoard,
    /// Déclencher la lecture d'un média.
    TriggerPlayback,
    /// Arrêter la lecture en cours.
    StopPlayback,
    /// Ajouter un média au board.
    AddMedia,
    /// Modifier ou supprimer un média que l'on a soi-même ajouté.
    EditOwnMedia,
    /// Modifier ou supprimer n'importe quel média.
    EditAnyMedia,
    /// Retirer, réordonner ou vider la file d'attente.
    ModerateQueue,
    /// Changer un rôle ou exclure un membre.
    ManageMembers,
    /// Créer ou révoquer des invitations.
    ManageInvites,
    /// Modifier les réglages de la guilde (dont la liaison Discord).
    ManageSettings,
    /// Purger les données de la guilde.
    PurgeGuild,
}

impl GuildRole {
    /// `true` si le rôle autorise `permission`.
    pub const fn can(self, permission: Permission) -> bool {
        match permission {
            Permission::ReceiveMedia | Permission::ViewQueue | Permission::ViewBoard => true,
            Permission::TriggerPlayback
            | Permission::StopPlayback
            | Permission::AddMedia
            | Permission::EditOwnMedia => matches!(self, Self::Admin | Self::Member),
            Permission::EditAnyMedia
            | Permission::ModerateQueue
            | Permission::ManageMembers
            | Permission::ManageInvites
            | Permission::ManageSettings
            | Permission::PurgeGuild => matches!(self, Self::Admin),
        }
    }

    /// `true` si le rôle peut consulter le board, compte tenu du réglage
    /// `guests_can_view_board` de la guilde.
    pub const fn can_view_board(self, guests_can_view_board: bool) -> bool {
        match self {
            Self::Guest => guests_can_view_board,
            Self::Admin | Self::Member => true,
        }
    }

    /// Mode de session WS équivalent (parité protocole v2).
    pub const fn session_mode(self) -> OverlaySessionMode {
        match self {
            Self::Guest => OverlaySessionMode::InviteReadOnly,
            Self::Admin | Self::Member => OverlaySessionMode::Normal,
        }
    }

    /// Rôle accordé par un code d'appairage legacy (émis depuis Discord).
    pub const fn from_pairing_mode(mode: PairingMode) -> Self {
        match mode {
            PairingMode::Normal => Self::Member,
            PairingMode::InviteReadOnly => Self::Guest,
        }
    }
}

impl JobStatus {
    /// `true` pour les états terminaux (`DONE`, `FAILED`).
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }

    /// `true` si la transition `from -> self` est autorisée par le scheduler.
    ///
    /// Le scheduler v1 ne faisait que `PENDING -> PLAYING -> DONE|FAILED`,
    /// plus les reprises `PLAYING -> PENDING` (requeue) et
    /// `PENDING -> FAILED` (média indisponible, cf.
    /// `playbackScheduler.ts` `dispatchPendingJob`).
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Pending, Self::Playing)
                | (Self::Pending, Self::Failed)
                | (Self::Playing, Self::Done)
                | (Self::Playing, Self::Failed)
                | (Self::Playing, Self::Pending)
                | (Self::Pending, Self::Pending)
                | (Self::Done, Self::Done)
                | (Self::Failed, Self::Failed)
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{JobStatus, MediaKind, MediaStatus, OverlaySessionMode, PairingMode};

    #[test]
    fn conversions_base_et_fil() {
        for kind in MediaKind::ALL {
            assert_eq!(MediaKind::from_db_str(kind.as_db_str()).unwrap(), *kind);
            assert_eq!(MediaKind::from_wire_str(kind.as_wire_str()).unwrap(), *kind);
        }

        assert_eq!(MediaKind::Image.as_db_str(), "IMAGE");
        assert_eq!(MediaKind::Image.as_wire_str(), "image");
        assert_eq!(MediaStatus::Processing.as_db_str(), "PROCESSING");
        assert_eq!(JobStatus::Playing.as_db_str(), "PLAYING");
        assert_eq!(
            OverlaySessionMode::InviteReadOnly.as_db_str(),
            "INVITE_READ_ONLY"
        );
        assert_eq!(
            OverlaySessionMode::InviteReadOnly.as_wire_str(),
            "invite_read_only"
        );
        assert_eq!(PairingMode::Normal.as_db_str(), "NORMAL");
    }

    #[test]
    fn valeurs_inconnues_rejetees() {
        assert!(MediaKind::from_db_str("image").is_err());
        assert!(MediaKind::from_db_str("GIF").is_err());
        assert!(JobStatus::from_wire_str("PLAYING").is_err());
    }

    #[test]
    fn permissions_par_role() {
        use super::{GuildRole, Permission};

        for role in GuildRole::ALL {
            assert!(role.can(Permission::ReceiveMedia));
            assert!(role.can(Permission::ViewQueue));
        }
        assert!(!GuildRole::Guest.can(Permission::TriggerPlayback));
        assert!(!GuildRole::Guest.can(Permission::AddMedia));
        assert!(GuildRole::Member.can(Permission::TriggerPlayback));
        assert!(GuildRole::Member.can(Permission::EditOwnMedia));
        assert!(!GuildRole::Member.can(Permission::EditAnyMedia));
        assert!(!GuildRole::Member.can(Permission::ManageMembers));
        assert!(GuildRole::Admin.can(Permission::ManageMembers));
        assert!(GuildRole::Admin.can(Permission::PurgeGuild));

        assert!(!GuildRole::Guest.can_view_board(false));
        assert!(GuildRole::Guest.can_view_board(true));
        assert!(GuildRole::Member.can_view_board(false));

        assert_eq!(
            GuildRole::Guest.session_mode(),
            OverlaySessionMode::InviteReadOnly
        );
        assert_eq!(GuildRole::Admin.session_mode(), OverlaySessionMode::Normal);
        assert_eq!(
            GuildRole::from_pairing_mode(PairingMode::InviteReadOnly),
            GuildRole::Guest
        );
        assert_eq!(GuildRole::Admin.as_db_str(), "ADMIN");
        assert_eq!(GuildRole::Guest.as_wire_str(), "guest");
    }

    #[test]
    fn job_status_terminal_et_transitions() {
        assert!(!JobStatus::Pending.is_terminal());
        assert!(!JobStatus::Playing.is_terminal());
        assert!(JobStatus::Done.is_terminal());
        assert!(JobStatus::Failed.is_terminal());

        assert!(JobStatus::Pending.can_transition_to(JobStatus::Playing));
        assert!(JobStatus::Playing.can_transition_to(JobStatus::Done));
        assert!(JobStatus::Playing.can_transition_to(JobStatus::Pending));
        assert!(!JobStatus::Done.can_transition_to(JobStatus::Playing));
        assert!(!JobStatus::Pending.can_transition_to(JobStatus::Done));
    }
}
