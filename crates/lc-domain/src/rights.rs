//! Configurable rights per surface: overlay, browser extension and Discord bot.
//!
//! The persisted overrides live in `role_rights`; a missing row means "use the
//! built-in default". Defaults reproduce the historical hard-coded rules so a
//! fresh database behaves exactly like the previous code:
//!
//! - `overlay`: [`GuildRole::can`] for every [`Permission`] variant;
//! - `extension`: pairing and sending are open to members and admins;
//! - `discord`: the old `lc_discord::rights::requirement` table (`msg`, `tts`
//!   and `mediaAdd` for members and admins, `stop` and `overlays` for admins,
//!   `pairCode` and `info` for everyone including unlinked accounts).
//!
//! Fixed cells cannot be configured: the whole `admin` column of `overlay`
//! and `extension`, `overlay.receiveMedia` for every role and `discord.info` for
//! every role always answer `true`. [`RightsTable::allowed`] enforces them even
//! if a corrupt database contains an override for them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::enums::{GuildRole, Permission};

/// Surface whose rights are configured.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// Desktop overlay (`/overlay/**`).
    Overlay,
    /// Browser extension (`POST /ingest`).
    Extension,
    /// Discord bot commands.
    Discord,
}

impl Surface {
    /// Every surface, in the order used by `GET /admin/api/rights`.
    pub const ALL: &'static [Self] = &[Self::Overlay, Self::Extension, Self::Discord];

    /// Stable identifier (`overlay`, `extension`, `discord`).
    pub const fn id(self) -> &'static str {
        match self {
            Self::Overlay => "overlay",
            Self::Extension => "extension",
            Self::Discord => "discord",
        }
    }

    /// Parses a stable identifier.
    pub fn from_id(raw: &str) -> Option<Self> {
        match raw {
            "overlay" => Some(Self::Overlay),
            "extension" => Some(Self::Extension),
            "discord" => Some(Self::Discord),
            _ => None,
        }
    }

    /// Roles accepted as a column of this surface, in display order.
    pub const fn columns(self) -> &'static [RightsRole] {
        match self {
            Self::Overlay | Self::Extension => {
                &[RightsRole::Guest, RightsRole::Member, RightsRole::Admin]
            }
            Self::Discord => &[
                RightsRole::None,
                RightsRole::Guest,
                RightsRole::Member,
                RightsRole::Admin,
            ],
        }
    }

    /// Actions of this surface, in display order.
    pub const fn actions(self) -> &'static [Action] {
        match self {
            Self::Overlay => OVERLAY_ACTIONS,
            Self::Extension => EXTENSION_ACTIONS,
            Self::Discord => DISCORD_ACTIONS,
        }
    }
}

/// Role a right is configured for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RightsRole {
    /// Unlinked account (Discord only).
    None,
    /// Guest / read-only member.
    Guest,
    /// Ordinary member.
    Member,
    /// Guild administrator.
    Admin,
}

impl RightsRole {
    /// Every role, in display order.
    pub const ALL: &'static [Self] = &[Self::None, Self::Guest, Self::Member, Self::Admin];

    /// Stable identifier (`none`, `guest`, `member`, `admin`).
    pub const fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Guest => "guest",
            Self::Member => "member",
            Self::Admin => "admin",
        }
    }

    /// Parses a stable identifier.
    pub fn from_id(raw: &str) -> Option<Self> {
        match raw {
            "none" => Some(Self::None),
            "guest" => Some(Self::Guest),
            "member" => Some(Self::Member),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }

    /// Guild role equivalent (`None` for the unlinked role).
    pub const fn guild_role(self) -> Option<GuildRole> {
        match self {
            Self::None => None,
            Self::Guest => Some(GuildRole::Guest),
            Self::Member => Some(GuildRole::Member),
            Self::Admin => Some(GuildRole::Admin),
        }
    }
}

impl From<GuildRole> for RightsRole {
    fn from(role: GuildRole) -> Self {
        match role {
            GuildRole::Guest => Self::Guest,
            GuildRole::Member => Self::Member,
            GuildRole::Admin => Self::Admin,
        }
    }
}

/// Permission an overlay [`Action`] maps to.
const OVERLAY_ACTIONS: &[Action] = &[
    Action::ReceiveMedia,
    Action::ViewQueue,
    Action::ViewBoard,
    Action::TriggerPlayback,
    Action::StopPlayback,
    Action::AddMedia,
    Action::EditOwnMedia,
    Action::EditAnyMedia,
    Action::ModerateQueue,
    Action::ManageMembers,
    Action::ManageInvites,
    Action::ManageSettings,
    Action::PurgeGuild,
];

/// Actions of the browser extension.
const EXTENSION_ACTIONS: &[Action] = &[Action::Pair, Action::Send];

/// Actions of the Discord bot (excluding `/lier`, not configurable).
const DISCORD_ACTIONS: &[Action] = &[
    Action::Msg,
    Action::Tts,
    Action::MediaAdd,
    Action::Stop,
    Action::Overlays,
    Action::PairCode,
    Action::Info,
];

/// One configurable action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    /// Receive the media played on the overlay.
    ReceiveMedia,
    /// Read the guild queue.
    ViewQueue,
    /// Read the media board.
    ViewBoard,
    /// Start a playback.
    TriggerPlayback,
    /// Stop the current playback.
    StopPlayback,
    /// Add a media to the board.
    AddMedia,
    /// Edit or delete a media the member added.
    EditOwnMedia,
    /// Edit or delete any media.
    EditAnyMedia,
    /// Remove, reorder or clear the queue.
    ModerateQueue,
    /// Change a member role or remove a member.
    ManageMembers,
    /// Create or revoke invites.
    ManageInvites,
    /// Change the guild settings.
    ManageSettings,
    /// Purge the guild data.
    PurgeGuild,
    /// Pair a browser extension.
    Pair,
    /// Send from the browser extension.
    Send,
    /// `/msg`, `/cmsg`, `/dire`, `/cdire` (start a playback).
    Msg,
    /// `/dire`, `/cdire` (text to speech).
    Tts,
    /// `/media-add`.
    MediaAdd,
    /// `/stop`.
    Stop,
    /// `/overlays`.
    Overlays,
    /// `/pair-code`.
    PairCode,
    /// `/alive`, `/help`, `/info`.
    Info,
}

impl Action {
    /// Stable identifier used by the API and the `role_rights.action` column.
    pub const fn id(self) -> &'static str {
        match self {
            Self::ReceiveMedia => "receiveMedia",
            Self::ViewQueue => "viewQueue",
            Self::ViewBoard => "viewBoard",
            Self::TriggerPlayback => "triggerPlayback",
            Self::StopPlayback => "stopPlayback",
            Self::AddMedia => "addMedia",
            Self::EditOwnMedia => "editOwnMedia",
            Self::EditAnyMedia => "editAnyMedia",
            Self::ModerateQueue => "moderateQueue",
            Self::ManageMembers => "manageMembers",
            Self::ManageInvites => "manageInvites",
            Self::ManageSettings => "manageSettings",
            Self::PurgeGuild => "purgeGuild",
            Self::Pair => "pair",
            Self::Send => "send",
            Self::Msg => "msg",
            Self::Tts => "tts",
            Self::MediaAdd => "mediaAdd",
            Self::Stop => "stop",
            Self::Overlays => "overlays",
            Self::PairCode => "pairCode",
            Self::Info => "info",
        }
    }

    /// Parses a stable identifier.
    pub fn from_id(raw: &str) -> Option<Self> {
        Some(match raw {
            "receiveMedia" => Self::ReceiveMedia,
            "viewQueue" => Self::ViewQueue,
            "viewBoard" => Self::ViewBoard,
            "triggerPlayback" => Self::TriggerPlayback,
            "stopPlayback" => Self::StopPlayback,
            "addMedia" => Self::AddMedia,
            "editOwnMedia" => Self::EditOwnMedia,
            "editAnyMedia" => Self::EditAnyMedia,
            "moderateQueue" => Self::ModerateQueue,
            "manageMembers" => Self::ManageMembers,
            "manageInvites" => Self::ManageInvites,
            "manageSettings" => Self::ManageSettings,
            "purgeGuild" => Self::PurgeGuild,
            "pair" => Self::Pair,
            "send" => Self::Send,
            "msg" => Self::Msg,
            "tts" => Self::Tts,
            "mediaAdd" => Self::MediaAdd,
            "stop" => Self::Stop,
            "overlays" => Self::Overlays,
            "pairCode" => Self::PairCode,
            "info" => Self::Info,
            _ => return None,
        })
    }

    /// Overlay permission matching this action, if it is an overlay action.
    pub const fn overlay_permission(self) -> Option<Permission> {
        match self {
            Self::ReceiveMedia => Some(Permission::ReceiveMedia),
            Self::ViewQueue => Some(Permission::ViewQueue),
            Self::ViewBoard => Some(Permission::ViewBoard),
            Self::TriggerPlayback => Some(Permission::TriggerPlayback),
            Self::StopPlayback => Some(Permission::StopPlayback),
            Self::AddMedia => Some(Permission::AddMedia),
            Self::EditOwnMedia => Some(Permission::EditOwnMedia),
            Self::EditAnyMedia => Some(Permission::EditAnyMedia),
            Self::ModerateQueue => Some(Permission::ModerateQueue),
            Self::ManageMembers => Some(Permission::ManageMembers),
            Self::ManageInvites => Some(Permission::ManageInvites),
            Self::ManageSettings => Some(Permission::ManageSettings),
            Self::PurgeGuild => Some(Permission::PurgeGuild),
            Self::Pair
            | Self::Send
            | Self::Msg
            | Self::Tts
            | Self::MediaAdd
            | Self::Stop
            | Self::Overlays
            | Self::PairCode
            | Self::Info => None,
        }
    }

    /// Overlay action of a permission.
    pub const fn from_overlay_permission(permission: Permission) -> Self {
        match permission {
            Permission::ReceiveMedia => Self::ReceiveMedia,
            Permission::ViewQueue => Self::ViewQueue,
            Permission::ViewBoard => Self::ViewBoard,
            Permission::TriggerPlayback => Self::TriggerPlayback,
            Permission::StopPlayback => Self::StopPlayback,
            Permission::AddMedia => Self::AddMedia,
            Permission::EditOwnMedia => Self::EditOwnMedia,
            Permission::EditAnyMedia => Self::EditAnyMedia,
            Permission::ModerateQueue => Self::ModerateQueue,
            Permission::ManageMembers => Self::ManageMembers,
            Permission::ManageInvites => Self::ManageInvites,
            Permission::ManageSettings => Self::ManageSettings,
            Permission::PurgeGuild => Self::PurgeGuild,
        }
    }
}

/// Effective rights table: defaults, plus the explicit overrides read from the
/// database, plus the `discordAdminIsAdmin` flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RightsTable {
    overrides: BTreeMap<(Surface, Action, RightsRole), bool>,
    discord_admin_is_admin: bool,
}

impl Default for RightsTable {
    fn default() -> Self {
        Self::new()
    }
}

impl RightsTable {
    /// Empty table: only the built-in defaults, `discordAdminIsAdmin` enabled
    /// (the historical behaviour).
    pub fn new() -> Self {
        Self {
            overrides: BTreeMap::new(),
            discord_admin_is_admin: true,
        }
    }

    /// `true` if `role` may perform `action` on `surface`.
    ///
    /// Unknown surface/action/role combinations are refused. Fixed cells always
    /// answer their fixed value.
    pub fn allowed(&self, surface: Surface, action: Action, role: RightsRole) -> bool {
        if !surface.actions().contains(&action) || !surface.columns().contains(&role) {
            return false;
        }
        if Self::is_fixed(surface, action, role) {
            return true;
        }
        match self.overrides.get(&(surface, action, role)) {
            Some(allowed) => *allowed,
            None => Self::default_allowed(surface, action, role),
        }
    }

    /// Built-in value of a cell, ignoring overrides.
    pub const fn default_allowed(surface: Surface, action: Action, role: RightsRole) -> bool {
        match surface {
            Surface::Overlay => match action.overlay_permission() {
                Some(permission) => match role.guild_role() {
                    Some(guild_role) => guild_role.can(permission),
                    None => false,
                },
                None => false,
            },
            Surface::Extension => matches!(
                (action, role),
                (
                    Action::Pair | Action::Send,
                    RightsRole::Admin | RightsRole::Member
                )
            ),
            Surface::Discord => match action {
                Action::Msg | Action::Tts | Action::MediaAdd => {
                    matches!(role, RightsRole::Admin | RightsRole::Member)
                }
                Action::Stop | Action::Overlays => matches!(role, RightsRole::Admin),
                Action::PairCode | Action::Info => true,
                _ => false,
            },
        }
    }

    /// `true` if a cell is frozen by the contract and cannot be overridden.
    pub fn is_fixed(surface: Surface, action: Action, role: RightsRole) -> bool {
        if !surface.actions().contains(&action) || !surface.columns().contains(&role) {
            return false;
        }
        match (surface, role) {
            (Surface::Overlay | Surface::Extension, RightsRole::Admin) => true,
            (Surface::Overlay, _) if action == Action::ReceiveMedia => true,
            (Surface::Discord, _) if action == Action::Info => true,
            _ => false,
        }
    }

    /// Records an explicit override.
    ///
    /// Fixed cells are accepted here (the table still answers their fixed
    /// value); the admin route validates and refuses them before calling this.
    pub fn set_override(
        &mut self,
        surface: Surface,
        action: Action,
        role: RightsRole,
        allowed: bool,
    ) {
        self.overrides.insert((surface, action, role), allowed);
    }

    /// Explicit override of a cell, when one is stored.
    pub fn override_of(&self, surface: Surface, action: Action, role: RightsRole) -> Option<bool> {
        self.overrides.get(&(surface, action, role)).copied()
    }

    /// Number of explicit overrides.
    pub fn override_count(&self) -> usize {
        self.overrides.len()
    }

    /// Removes every override (back to the built-in defaults).
    pub fn clear_overrides(&mut self) {
        self.overrides.clear();
    }

    /// `true` when a Discord server administrator counts as the `admin` role.
    pub fn discord_admin_is_admin(&self) -> bool {
        self.discord_admin_is_admin
    }

    /// Sets the `discordAdminIsAdmin` flag.
    pub fn set_discord_admin_is_admin(&mut self, value: bool) {
        self.discord_admin_is_admin = value;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        Action, RightsRole, RightsTable, Surface, DISCORD_ACTIONS, EXTENSION_ACTIONS,
        OVERLAY_ACTIONS,
    };
    use crate::enums::{GuildRole, Permission};

    /// Old `lc_discord::rights::requirement` behaviour, one boolean per action
    /// and role (`true` = the requirement let the author through, provided a
    /// linked member exists where the old rule asked for one).
    fn old_discord_requirement(action: Action, role: RightsRole) -> bool {
        match action {
            Action::Msg | Action::Tts | Action::MediaAdd => {
                matches!(role, RightsRole::Admin | RightsRole::Member)
            }
            Action::Stop | Action::Overlays => matches!(role, RightsRole::Admin),
            Action::PairCode | Action::Info => true,
            _ => false,
        }
    }

    #[test]
    fn ids_round_trip() {
        for surface in Surface::ALL {
            assert_eq!(Surface::from_id(surface.id()), Some(*surface));
        }
        for role in RightsRole::ALL {
            assert_eq!(RightsRole::from_id(role.id()), Some(*role));
        }
        for action in OVERLAY_ACTIONS
            .iter()
            .chain(EXTENSION_ACTIONS)
            .chain(DISCORD_ACTIONS)
        {
            assert_eq!(Action::from_id(action.id()), Some(*action));
        }
        assert_eq!(Surface::from_id("inconnue"), None);
        assert_eq!(RightsRole::from_id("owner"), None);
        assert_eq!(Action::from_id("unknown"), None);
    }

    #[test]
    fn actions_and_columns_match_the_contract() {
        assert_eq!(
            Surface::Overlay.columns(),
            &[RightsRole::Guest, RightsRole::Member, RightsRole::Admin]
        );
        assert_eq!(
            Surface::Discord.columns(),
            &[
                RightsRole::None,
                RightsRole::Guest,
                RightsRole::Member,
                RightsRole::Admin
            ]
        );
        assert_eq!(Surface::Overlay.actions(), OVERLAY_ACTIONS);
        assert_eq!(Surface::Extension.actions(), EXTENSION_ACTIONS);
        assert_eq!(Surface::Discord.actions(), DISCORD_ACTIONS);
        assert_eq!(OVERLAY_ACTIONS.len(), 13);
        assert_eq!(
            OVERLAY_ACTIONS
                .iter()
                .map(|action| action.id())
                .collect::<Vec<_>>(),
            vec![
                "receiveMedia",
                "viewQueue",
                "viewBoard",
                "triggerPlayback",
                "stopPlayback",
                "addMedia",
                "editOwnMedia",
                "editAnyMedia",
                "moderateQueue",
                "manageMembers",
                "manageInvites",
                "manageSettings",
                "purgeGuild",
            ]
        );
        assert_eq!(
            EXTENSION_ACTIONS
                .iter()
                .map(|action| action.id())
                .collect::<Vec<_>>(),
            vec!["pair", "send"]
        );
        assert_eq!(
            DISCORD_ACTIONS
                .iter()
                .map(|action| action.id())
                .collect::<Vec<_>>(),
            vec!["msg", "tts", "mediaAdd", "stop", "overlays", "pairCode", "info"]
        );
    }

    #[test]
    fn overlay_defaults_equal_guild_role_can_for_every_permission() {
        let table = RightsTable::new();
        for permission in [
            Permission::ReceiveMedia,
            Permission::ViewQueue,
            Permission::ViewBoard,
            Permission::TriggerPlayback,
            Permission::StopPlayback,
            Permission::AddMedia,
            Permission::EditOwnMedia,
            Permission::EditAnyMedia,
            Permission::ModerateQueue,
            Permission::ManageMembers,
            Permission::ManageInvites,
            Permission::ManageSettings,
            Permission::PurgeGuild,
        ] {
            let action = Action::from_overlay_permission(permission);
            for guild_role in GuildRole::ALL {
                let role = RightsRole::from(*guild_role);
                assert_eq!(
                    table.allowed(Surface::Overlay, action, role),
                    guild_role.can(permission),
                    "{:?} for {:?}",
                    permission,
                    guild_role
                );
                assert_eq!(
                    RightsTable::default_allowed(Surface::Overlay, action, role),
                    guild_role.can(permission)
                );
            }
        }
    }

    #[test]
    fn extension_defaults_are_member_and_admin() {
        let table = RightsTable::new();
        for action in EXTENSION_ACTIONS {
            for role in Surface::Extension.columns() {
                assert_eq!(
                    table.allowed(Surface::Extension, *action, *role),
                    matches!(role, RightsRole::Member | RightsRole::Admin),
                    "{} for {}",
                    action.id(),
                    role.id()
                );
            }
        }
    }

    #[test]
    fn discord_defaults_equal_the_old_requirement_table() {
        let table = RightsTable::new();
        for action in DISCORD_ACTIONS {
            for role in Surface::Discord.columns() {
                assert_eq!(
                    table.allowed(Surface::Discord, *action, *role),
                    old_discord_requirement(*action, *role),
                    "{} for {}",
                    action.id(),
                    role.id()
                );
            }
        }
    }

    #[test]
    fn fixed_cells_are_frozen() {
        let mut table = RightsTable::new();
        // Fixed cells are all `true`, whatever the role and the surface.
        for surface in [Surface::Overlay, Surface::Extension] {
            for action in surface.actions() {
                assert!(RightsTable::is_fixed(surface, *action, RightsRole::Admin));
                assert!(table.allowed(surface, *action, RightsRole::Admin));
            }
        }
        for role in Surface::Overlay.columns() {
            assert!(RightsTable::is_fixed(
                Surface::Overlay,
                Action::ReceiveMedia,
                *role
            ));
        }
        for role in Surface::Discord.columns() {
            assert!(RightsTable::is_fixed(Surface::Discord, Action::Info, *role));
        }

        // A stored override cannot flip a fixed cell.
        table.set_override(
            Surface::Overlay,
            Action::ReceiveMedia,
            RightsRole::Guest,
            false,
        );
        table.set_override(Surface::Extension, Action::Pair, RightsRole::Admin, false);
        table.set_override(Surface::Discord, Action::Info, RightsRole::None, false);
        assert!(table.allowed(Surface::Overlay, Action::ReceiveMedia, RightsRole::Guest));
        assert!(table.allowed(Surface::Extension, Action::Pair, RightsRole::Admin));
        assert!(table.allowed(Surface::Discord, Action::Info, RightsRole::None));

        // Configurable cells are not fixed.
        assert!(!RightsTable::is_fixed(
            Surface::Overlay,
            Action::TriggerPlayback,
            RightsRole::Guest
        ));
        assert!(!RightsTable::is_fixed(
            Surface::Extension,
            Action::Send,
            RightsRole::Member
        ));
        assert!(!RightsTable::is_fixed(
            Surface::Discord,
            Action::Msg,
            RightsRole::Guest
        ));
    }

    #[test]
    fn overrides_replace_the_defaults_only_for_their_cell() {
        let mut table = RightsTable::new();
        assert!(!table.allowed(Surface::Overlay, Action::TriggerPlayback, RightsRole::Guest));
        assert!(table.allowed(
            Surface::Overlay,
            Action::TriggerPlayback,
            RightsRole::Member
        ));

        table.set_override(
            Surface::Overlay,
            Action::TriggerPlayback,
            RightsRole::Guest,
            true,
        );
        assert!(table.allowed(Surface::Overlay, Action::TriggerPlayback, RightsRole::Guest));
        // The default is untouched.
        assert!(!RightsTable::default_allowed(
            Surface::Overlay,
            Action::TriggerPlayback,
            RightsRole::Guest
        ));
        assert_eq!(table.override_count(), 1);
        assert_eq!(
            table.override_of(Surface::Overlay, Action::TriggerPlayback, RightsRole::Guest),
            Some(true)
        );
        assert_eq!(
            table.override_of(Surface::Overlay, Action::AddMedia, RightsRole::Guest),
            None
        );

        // Turn a default-allowed cell off.
        table.set_override(Surface::Discord, Action::Msg, RightsRole::Member, false);
        assert!(!table.allowed(Surface::Discord, Action::Msg, RightsRole::Member));
        assert!(table.allowed(Surface::Discord, Action::Msg, RightsRole::Admin));

        table.clear_overrides();
        assert_eq!(table, RightsTable::new());
    }

    #[test]
    fn unknown_combinations_are_refused() {
        let table = RightsTable::new();
        // `pair` is not an overlay action, `none` is not an overlay column and
        // `msg` is not an overlay action.
        assert!(!table.allowed(Surface::Overlay, Action::Pair, RightsRole::Member));
        assert!(!table.allowed(Surface::Overlay, Action::Msg, RightsRole::Guest));
        assert!(!table.allowed(Surface::Overlay, Action::TriggerPlayback, RightsRole::None));
        assert!(!table.allowed(Surface::Discord, Action::Pair, RightsRole::Guest));
        assert!(!RightsTable::is_fixed(
            Surface::Discord,
            Action::Pair,
            RightsRole::Admin
        ));
    }

    #[test]
    fn discord_admin_flag_defaults_to_true_and_can_be_disabled() {
        let mut table = RightsTable::new();
        assert!(table.discord_admin_is_admin());
        table.set_discord_admin_is_admin(false);
        assert!(!table.discord_admin_is_admin());
        assert!(RightsTable::default().discord_admin_is_admin());
    }

    #[test]
    fn cells_outside_a_surface_are_never_allowed_even_with_an_override() {
        let mut table = RightsTable::new();
        // Valid role, action of another surface.
        table.set_override(Surface::Extension, Action::Stop, RightsRole::Admin, true);
        assert!(!table.allowed(Surface::Extension, Action::Stop, RightsRole::Admin));
        // Valid action, role that is not a column of the surface.
        table.set_override(Surface::Overlay, Action::ViewQueue, RightsRole::None, true);
        assert!(!table.allowed(Surface::Overlay, Action::ViewQueue, RightsRole::None));
    }

    #[test]
    fn cells_outside_a_surface_are_never_fixed() {
        // Admin columns of Overlay/Extension are fixed, but not for a foreign action.
        assert!(!RightsTable::is_fixed(
            Surface::Extension,
            Action::Info,
            RightsRole::Admin
        ));
        // `receiveMedia` is fixed for the overlay columns only.
        assert!(!RightsTable::is_fixed(
            Surface::Overlay,
            Action::ReceiveMedia,
            RightsRole::None
        ));
        assert!(RightsTable::is_fixed(
            Surface::Overlay,
            Action::ReceiveMedia,
            RightsRole::Guest
        ));
    }

    #[test]
    fn override_count_follows_the_overrides() {
        let mut table = RightsTable::new();
        assert_eq!(table.override_count(), 0);
        table.set_override(
            Surface::Overlay,
            Action::ViewBoard,
            RightsRole::Guest,
            false,
        );
        table.set_override(Surface::Discord, Action::Tts, RightsRole::Guest, true);
        assert_eq!(table.override_count(), 2);
        table.clear_overrides();
        assert_eq!(table.override_count(), 0);
    }
}
