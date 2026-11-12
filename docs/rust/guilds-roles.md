# Guildes, rôles et invitations

Ce document décrit les guildes indépendantes de Discord, les rôles et les invitations, tels qu'implémentés dans le serveur Rust et l'overlay Tauri. Sources principales : `crates/lc-db/migrations/0003_guild_members.sql`, `crates/lc-domain/src/{enums,entities,ids}.rs`, les repositories `lc-db`, `apps/lc-server/src/http/overlay/*`, `apps/lc-server/src/http/admin/guilds.rs` et `apps/lc-overlay/src/*`.

## Vue d'ensemble

Une guilde est identifiée par un `GuildId` numérique. Deux origines coexistent :

- `guilds.standalone = 0` : l'identifiant est le snowflake du serveur Discord. Les guildes existantes et celles que les flux Discord créent encore à la volée gardent ce sens.
- `guilds.standalone = 1` : la guilde a été créée hors Discord, depuis le panneau serveur (`POST /admin/api/guilds`). Son identifiant est généré localement avec le même format qu'un snowflake.

Une guilde indépendante peut être liée à un serveur Discord par `guilds.discord_guild_id` (voir « Liaison Discord »).

L'adhésion se fait par code. `POST /overlay/pair/consume` cherche le code dans `guild_invites` (invitations porteuses d'un rôle), puis dans `pairing_codes` (codes legacy émis depuis Discord). Chaque appairage crée un `GuildMember` ou rattache l'overlay à un membre existant ; `overlay_clients.member_id` porte ce rattachement.

Trois rôles existent : `admin`, `member`, `guest`. Les permissions sont décidées par `GuildRole::can` (`crates/lc-domain/src/enums.rs`), et la visibilité du board par `GuildRole::can_view_board` ; les handlers s'appuient sur ces méthodes plutôt que sur des listes de rôles. Le rôle vient toujours de la base, jamais du corps de la requête.

Côté overlay, `config.json` contient une liste d'adhésions (`memberships`). L'application maintient un superviseur WebSocket par guilde rejointe et arbitre l'affichage avec une file locale, car un seul média occupe l'écran à la fois.

## Modèle de données

### Colonnes ajoutées par la migration 0003

| Table | Colonne | Type et contrainte | Sens |
|-------|---------|--------------------|------|
| `guilds` | `standalone` | `INTEGER NOT NULL DEFAULT 0`, `CHECK IN (0,1)` | `1` : créée hors Discord ; `0` : identifiant = snowflake Discord |
| `guilds` | `discord_guild_id` | `TEXT`, index unique partiel sur les valeurs non nulles | Serveur Discord lié à une guilde indépendante |
| `guilds` | `guests_can_view_board` | `INTEGER NOT NULL DEFAULT 1`, `CHECK IN (0,1)` | Autorise les invités à consulter le meme board |
| `guilds` | `created_at` | `INTEGER` nullable | `NULL` pour une guilde antérieure à la migration |
| `overlay_clients` | `member_id` | `TEXT REFERENCES guild_members(id)` | Membre propriétaire de l'overlay |
| `meme_board_items` | `created_by_member_id` | `TEXT` | Auteur d'un mème, pour le droit « modifier ses propres mèmes » |

La migration crée les index `idx_guilds_discord_guild_id`, `idx_guild_members_guild`, `idx_overlay_clients_member` et `idx_guild_invites_guild`.

### Tables `guild_members` et `guild_invites`

| Table | Colonne | Type et contrainte |
|-------|---------|--------------------|
| `guild_members` | `id` | `TEXT PRIMARY KEY` (UUID v4, type `GuildMemberId`) |
| `guild_members` | `guild_id` | `TEXT NOT NULL` |
| `guild_members` | `display_name` | `TEXT NOT NULL` |
| `guild_members` | `avatar` | `TEXT` nullable |
| `guild_members` | `role` | `TEXT NOT NULL`, `CHECK IN ('ADMIN','MEMBER','GUEST')` |
| `guild_members` | `created_at` | `INTEGER NOT NULL` (millisecondes epoch) |
| `guild_members` | `removed_at` | `INTEGER` nullable ; `NULL` = membre actif |
| `guild_invites` | `code` | `TEXT PRIMARY KEY` (normalisé en majuscules) |
| `guild_invites` | `guild_id` | `TEXT NOT NULL` |
| `guild_invites` | `role` | `TEXT NOT NULL`, `CHECK IN ('ADMIN','MEMBER','GUEST')` |
| `guild_invites` | `member_id` | `TEXT REFERENCES guild_members(id) ON DELETE CASCADE` ; non nul = code « appareil » |
| `guild_invites` | `max_uses` | `INTEGER` nullable, `CHECK (max_uses IS NULL OR max_uses > 0)` |
| `guild_invites` | `use_count` | `INTEGER NOT NULL DEFAULT 0` |
| `guild_invites` | `created_by_member_id` | `TEXT` nullable ; `NULL` = émis par l'admin serveur |
| `guild_invites` | `expires_at` | `INTEGER` nullable ; `NULL` = jamais |
| `guild_invites` | `revoked_at` | `INTEGER` nullable ; `NULL` = non révoquée |
| `guild_invites` | `created_at` | `INTEGER NOT NULL` |

Un membre retiré n'est pas supprimé : `removed_at` marque le départ ou l'exclusion, et `GuildMember::is_active` teste `removed_at.is_none()`. Un membre peut posséder plusieurs overlays. Une invitation est utilisable si elle n'est ni révoquée, ni expirée, ni épuisée (`GuildInvite::is_usable_at`) ; `GuildInvitesRepository::consume` incrémente `use_count` dans la même instruction que les contrôles (`RETURNING *`), donc deux consommateurs concurrents ne dépassent pas `max_uses`.

Les colonnes `guild_id` de ces deux tables ne portent pas de clé étrangère vers `guilds`. Seuls `guild_invites.member_id` et `overlay_clients.member_id` référencent `guild_members`.

### Reprise des overlays existants

La migration insère un membre pour chaque ligne de `overlay_clients` :

- même identifiant : `guild_members.id = overlay_clients.id` ;
- `display_name` = `default_author_name` trimé s'il est non vide, sinon `label` ;
- `avatar` = `default_author_image` ;
- rôle `GUEST` si `session_mode = 'INVITE_READ_ONLY'`, sinon `MEMBER` ;
- `created_at` = date de création ; `removed_at` = `revoked_at` (un overlay révoqué donne un membre retiré).

`UPDATE overlay_clients SET member_id = id` rattache ensuite chaque overlay à son membre. Aucune guilde existante n'a d'admin après la migration : le premier code admin s'émet depuis le panneau serveur.

### Sens de `standalone` et `discord_guild_id`

`GuildId::generate` produit 19 chiffres commençant par `9` (`crates/lc-domain/src/ids.rs`) : même validation et même stockage qu'un snowflake. L'unicité réelle repose sur la clé primaire `guilds.id` ; l'appelant régénère en cas de conflit, et le panneau essaie jusqu'à 5 identifiants (`GUILD_ID_ATTEMPTS`).

`GuildsRepository::create_standalone` écrit `standalone = 1`. `GuildsRepository::link_discord` ne modifie que les lignes `standalone = 1` et renvoie `false` sinon ; un `discord_guild_id` déjà lié ailleurs déclenche un conflit d'unicité (`DomainError::Conflict`). `resolve_discord` privilégie une guilde indépendante liée à ce serveur, sinon il renvoie le snowflake lui-même. `link_discord` n'est appelé par aucun handler HTTP ni aucune commande de l'overlay dans le dépôt.

## Rôles et permissions

Les rôles sont stockés en majuscules (`ADMIN`, `MEMBER`, `GUEST`) et sérialisés en JSON en `snake_case` (`admin`, `member`, `guest`) via `GuildRole::as_db_str` et `as_wire_str` (`crates/lc-domain/src/enums.rs`). `GuildRole::can` décide des permissions ; le tableau reprend exactement ses branches.

| Permission | Admin | Membre | Invité |
|------------|:-----:|:------:|:------:|
| `ReceiveMedia` | oui | oui | oui |
| `ViewQueue` | oui | oui | oui |
| `ViewBoard` | oui | oui | oui |
| `TriggerPlayback` | oui | oui | non |
| `StopPlayback` | oui | oui | non |
| `AddMeme` | oui | oui | non |
| `EditOwnMeme` | oui | oui | non |
| `EditAnyMeme` | oui | non | non |
| `ModerateQueue` | oui | non | non |
| `ManageMembers` | oui | non | non |
| `ManageInvites` | oui | non | non |
| `ManageSettings` | oui | non | non |
| `PurgeGuild` | oui | non | non |

`ViewBoard` vaut `true` pour les trois rôles dans `can`. Pour un invité, la visibilité effective dépend en plus de `guests_can_view_board`.

### Rôle effectif d'un overlay

`AuthedOverlayClient::role` (`apps/lc-server/src/http/overlay/auth.rs`) utilise le rôle du membre rattaché. Sans membre, il déduit le rôle du mode de session persisté : `INVITE_READ_ONLY` donne `guest`, `normal` donne `member`. Un overlay sans membre ne peut donc jamais être admin.

`GuildRole::session_mode` fait la conversion inverse : `GUEST` donne `invite_read_only`, `ADMIN` et `MEMBER` donnent `normal`. `GuildRole::from_pairing_mode` convertit un code legacy : `NORMAL` donne `member`, `INVITE_READ_ONLY` donne `guest`.

### Règle du dernier admin

`GuildMembersRepository::set_role` (`crates/lc-db/src/repos/guild_members.rs`) applique la condition SQL suivante ; `remove` reprend la même condition sans la branche `? = 'ADMIN'`, puisqu'elle ne change pas le rôle :

```sql
WHERE id = ? AND removed_at IS NULL
  AND (role <> 'ADMIN' OR ? = 'ADMIN' OR
       (SELECT COUNT(*) FROM guild_members AS other
         WHERE other.guild_id = guild_members.guild_id
           AND other.role = 'ADMIN' AND other.removed_at IS NULL) > 1)
```

Une guilde conserve donc au moins un admin actif ; la vérification est faite dans l'instruction de mise à jour, donc deux rétrogradations concurrentes ne peuvent pas vider la guilde de ses admins. Si aucune ligne n'est modifiée, `classify` distingue `NotFound` (membre inconnu ou déjà retiré) de `LastAdmin` (membre actif protégé). Les handlers traduisent `LastAdmin` en `409 {"error":"last_admin"}` sur `PATCH` et `DELETE /overlay/admin/members/{memberId}`, ainsi que sur `POST /overlay/leave` pour le départ du dernier admin (`apps/lc-server/src/http/overlay/admin.rs`, `guild.rs`).

### Modification des mèmes (`can_edit`)

`can_edit` (`apps/lc-server/src/http/overlay/meme_board.rs`) décide si un client peut modifier ou supprimer un mème :

1. s'il a `EditAnyMeme` (admin) : `true` ;
2. sinon, s'il n'a pas `EditOwnMeme` (invité) : `false` ;
3. sinon, compare le membre propriétaire de l'overlay au membre auteur de `created_by_member_id` : égaux → `true` ; différents → `false` ; l'un des deux absent → `true`.

Le dernier cas couvre un mème antérieur aux rôles ou ajouté depuis Discord, ainsi qu'un overlay sans membre : ils restent modifiables par tout membre. `GET /overlay/meme-board/items` renvoie `createdByMemberId` et `canEdit` pour chaque élément.

### Visibilité du board pour les invités

`GuildRole::can_view_board` renvoie `guests_can_view_board` pour `GUEST`, et `true` pour `ADMIN` et `MEMBER`. `GuildsRepository::set_guests_can_view_board` crée la ligne de guilde si nécessaire ; la valeur par défaut est `1`. `ensure_board_visible` (`meme_board.rs`) ne consulte le réglage que pour un invité ; si la ligne `guilds` est absente, la valeur retenue est `true` (`access.is_none_or(...)`). Un invité non autorisé reçoit `403 {"error":"forbidden"}` sur `GET /overlay/meme-board/items` et `GET /overlay/meme-board/media/{assetId}`. Un admin ou un membre passe sans consulter le réglage.

## Rejoindre une guilde

### Les deux familles de codes

| Famille | Table | Rôle accordé | Usage | Expiration |
|---------|-------|--------------|-------|------------|
| Invitation | `guild_invites` | `ADMIN`, `MEMBER` ou `GUEST` porté par le code | `max_uses` optionnel, `use_count` atomique | `expires_at` optionnel |
| Code legacy Discord | `pairing_codes` | `NORMAL` → `member`, `INVITE_READ_ONLY` → `guest` | Usage unique (`used_at`) | `expires_at` optionnel ; `NULL` = permanent |

`consume_pairing` (`apps/lc-server/src/http/overlay/pair.rs`) essaie d'abord `guild_invites.consume`, puis `pairing_codes.consume`. Un code inconnu, déjà utilisé, révoqué, expiré ou épuisé donne `404 {"error":"pairing_code_invalid_or_expired"}`. Avant la recherche, le serveur supprime opportunément les codes de pairing expirés (`pairing_codes.delete_expired`) ; un échec de ce nettoyage est journalisé mais ne bloque pas l'appairage.

Normalisation : le code est trimé, les tirets sont retirés (`replace('-', "")`), puis validé par `PairingCode::parse` (1 à 32 caractères alphanumériques, passage en majuscules). Un code saisi `k7qx-m2pd` est accepté. Un code vide ou un JSON malformé donne `400 {"error":"invalid_payload"}` ; un code syntaxiquement invalide donne `404`.

### Code appareil

`POST /overlay/device-code` émet une invitation à usage unique, valable 10 minutes (`DEVICE_CODE_TTL_MINUTES`) : `member_id` et `created_by_member_id` = membre courant, `max_uses = 1`, rôle = rôle courant, aucun corps. Un second overlay qui consomme ce code n'ajoute pas de membre : `consume_pairing` retrouve le membre par `member_id`, exige qu'il soit actif et rattache le nouvel overlay. Le rôle utilisé est le rôle courant du membre, pas celui figé dans l'invitation ; un membre rétrogradé entre-temps obtient son rôle actuel. Le `displayName` fourni est ignoré pour un membre existant (`code_appareil_rattache_au_meme_membre`). Les codes appareil ne sont pas listés par `GET /overlay/admin/invites`, qui filtre les invitations dont `member_id` est non nul.

### Règle de révocation par libellé

Après avoir calculé le libellé du nouvel overlay, `consume_pairing` révoque d'éventuelles sessions précédentes :

- code legacy (`created_by_discord_user_id` renseigné) : liste tous les overlays de la guilde (`list_by_guild`) et révoque ceux qui sont actifs et portent le même libellé ;
- invitation : aucune révocation si le membre est nouveau ou si le client n'a pas fourni de `deviceName` ; sinon, liste les overlays actifs du même membre (`list_active_by_member`) et révoque ceux qui portent le même libellé.

Pour une invitation, un nom d'appareil ne peut donc pas déconnecter l'overlay d'un autre membre, et un libellé par défaut ne révoque pas le premier overlay lors de l'ajout d'un second.

À défaut de `deviceName`, le libellé est `Overlay-<authorName>` pour un code legacy, `Overlay-User` sinon, tronqué à 200 caractères. Le nom du nouveau membre suit l'ordre : `displayName` de la requête, `authorName` du code legacy, `deviceName`, puis `User`.

### Corps et réponse de `POST /overlay/pair/consume`

```json
{
  "code": "K7QXM2PD",
  "deviceName": "Poste salon",
  "displayName": "Alice",
  "avatar": "https://exemple.tld/avatar.png",
  "appVersion": "x.y.z",
  "sessionMode": "normal"
}
```

`code` est obligatoire. `deviceName` accepte les alias `label` et `clientName`. `displayName` et `avatar` alimentent le nouveau membre. `appVersion` sert à la journalisation. `sessionMode` est accepté puis ignoré : le rôle et le mode viennent du code persisté.

Réponse `200` :

```json
{
  "clientToken": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "clientId": "2f1c9e4a-3b7d-4c1e-9a2f-5d6e7f8a9b0c",
  "guildId": "9123456789012345678",
  "apiBaseUrl": "https://api.exemple.tld",
  "deviceName": "Poste salon",
  "authorName": null,
  "sessionMode": "normal",
  "role": "member",
  "memberId": "6a5b4c3d-2e1f-4a9b-8c7d-6e5f4a3b2c1d",
  "displayName": "Alice",
  "guildName": "Les Sardines",
  "protocolVersion": 2
}
```

`clientToken` est le jeton brut, affiché une seule fois ; seul son SHA-256 est stocké dans `overlay_clients.token_hash`. `apiBaseUrl` vient de `state.config.api_origin()`. `guildName` est `null` si aucune ligne `guilds` n'existe. `authorName` reprend uniquement le nom porté par un code legacy ; il vaut `null` pour une invitation, même si `displayName` est renseigné. `protocolVersion` vaut `lc_protocol::PROTOCOL_VERSION` (2).

Erreurs : `400 invalid_payload` (JSON malformé ou code absent), `404 pairing_code_invalid_or_expired`, `409 overlay_client_conflict` (collision de hash), `500 internal_error`, `503 database_unavailable`.

## Routes overlay

Toutes les routes authentifiées acceptent `Authorization: Bearer <token>`. Le repli `?token=` n'est accepté que sous `/overlay/meme-board/*` (`query_token` dans `auth.rs`) ; l'en-tête Bearer prime toujours. `AuthedOverlayClient` charge un client non révoqué, rafraîchit `last_seen_at` au mieux, puis charge le membre actif. Erreurs communes : `401 {"error":"unauthorized"}` si le jeton est absent, inconnu ou révoqué ; `503 {"error":"database_unavailable"}` sans base ; `500 {"error":"internal_error"}` sur échec base.

| Méthode | Chemin | Droit requis |
|---------|--------|--------------|
| `POST` | `/overlay/pair/consume` | public (code d'appairage) |
| `GET` | `/overlay/media/{assetId}` | URL signée HMAC (`clientId`, `exp`, `sig`) |
| `GET` | `/overlay/guild` | tout overlay authentifié |
| `PATCH` | `/overlay/profile` | tout membre (son propre profil) |
| `POST` | `/overlay/device-code` | tout membre |
| `POST` | `/overlay/leave` | tout overlay authentifié |
| `GET` | `/overlay/queue` | `ViewQueue` (tous les rôles) |
| `DELETE` | `/overlay/queue/{jobId}` | `ModerateQueue` (admin) |
| `DELETE` | `/overlay/queue` | `ModerateQueue` (admin) |
| `GET` | `/overlay/meme-board/items` | `ViewBoard` (invité selon `guests_can_view_board`) |
| `POST` | `/overlay/meme-board/items` | `AddMeme` (admin, membre) |
| `GET` | `/overlay/meme-board/media/{assetId}` | `ViewBoard` (invité selon `guests_can_view_board`) |
| `PATCH` | `/overlay/meme-board/items/{itemId}` | `EditAnyMeme` (admin) ou `EditOwnMeme` + auteur (membre) |
| `DELETE` | `/overlay/meme-board/items/{itemId}` | `EditAnyMeme` (admin) ou `EditOwnMeme` + auteur (membre) |
| `GET` | `/overlay/admin/members` | `ManageMembers` |
| `PATCH` | `/overlay/admin/members/{memberId}` | `ManageMembers` |
| `DELETE` | `/overlay/admin/members/{memberId}` | `ManageMembers` |
| `GET` | `/overlay/admin/invites` | `ManageInvites` |
| `POST` | `/overlay/admin/invites` | `ManageInvites` |
| `POST` | `/overlay/admin/invites/{code}/revoke` | `ManageInvites` |
| `PATCH` | `/overlay/admin/settings` | `ManageSettings` |

`GET /overlay/media/{assetId}` n'utilise pas de Bearer. Elle exige `clientId`, `exp` et `sig`, vérifie la signature HMAC-SHA256 de `assetId|clientId|exp`, l'expiration, l'existence d'un client actif et d'un job de lecture de sa guilde référençant le média. Réponses : flux `200`, `206` avec `Range`, `416` pour une plage invalide ; erreurs `403 media_url_invalid`, `403 media_url_expired`, `403 forbidden` (aucun job de la guilde), `404 media_not_found`, `404 media_not_found_on_disk`, `500`, `503`.

### Détail des routes authentifiées

| Route | Corps | Réponse 200 | Erreurs propres |
|-------|-------|-------------|-----------------|
| `GET /overlay/guild` | aucun | `{guild:{id,name,standalone,discordGuildId,guestsCanViewBoard,settings,connectedCount}, member, clientId, permissions}` | `500`, `503` |
| `PATCH /overlay/profile` | `{"displayName":"…","avatar":"…"}` ; `avatar` absent = conservé, `null` ou vide = effacé | `{updated:true, member:{id,displayName,avatar,role}}` | `400 invalid_payload`, `400 invalid_display_name`, `409 member_required`, `404 member_not_found` |
| `POST /overlay/device-code` | aucun | `{created:true, invite:{…}}` | `409 member_required`, `500` |
| `POST /overlay/leave` | aucun | `{left:true}` | `409 last_admin`, `404 member_not_found` |
| `GET /overlay/queue` | aucun | `{playing, pending, serverTime}` | aucun (les trois rôles ont `ViewQueue`) |
| `DELETE /overlay/queue/{jobId}` | aucun | `{removed:true ou false}` | `403 forbidden` (non-admin) |
| `DELETE /overlay/queue` | aucun | `{removedCount:n}` | `403 forbidden` (non-admin) |
| `GET /overlay/meme-board/items?q=&limit=&offset=` | aucun | `{total,limit,offset,items}` | `403 forbidden` (`guildId` différent ou board masqué aux invités) |
| `POST /overlay/meme-board/items` | `{"url":"…","title":"…","message":"…","forceRefresh":false}` ; `url` obligatoire | `{created, item}` (`created:false` si déjà épinglé) | `400 invalid_payload`, `403 read_only_session`, `422 media_ingestion_failed` |
| `GET /overlay/meme-board/media/{assetId}` | aucun | flux binaire (`200`, `206`, `416`) | `403 forbidden`, `404 media_not_found`, `404 media_not_found_on_disk` |
| `PATCH /overlay/meme-board/items/{itemId}` | `{"title":"…","message":"…"}` ; au moins un des deux | `{updated:true,item:{id,title,message}}` | `400 invalid_payload`, `403 read_only_session`, `403 forbidden`, `404 item_not_found` |
| `DELETE /overlay/meme-board/items/{itemId}` | aucun | `{deleted:true ou false}` | `403 read_only_session`, `403 forbidden` |
| `GET /overlay/admin/members` | aucun | `{count, items}` | `403 forbidden` |
| `PATCH /overlay/admin/members/{memberId}` | `{"role":"admin"}` (admin, member ou guest) | `{updated:true,member:{id,role}}` | `400 invalid_payload`, `400 invalid_role`, `403 forbidden`, `404 member_not_found`, `409 last_admin` |
| `DELETE /overlay/admin/members/{memberId}` | aucun | `{removed:true}` | `403 forbidden`, `404 member_not_found`, `409 last_admin` |
| `GET /overlay/admin/invites` | aucun | `{count, items}` | `403 forbidden` |
| `POST /overlay/admin/invites` | `{"role":"…","expiresInHours":24,"maxUses":10}` | `{created:true, invite}` | `400 invalid_payload`, `400 invalid_role`, `400 invalid_expiry`, `400 invalid_max_uses`, `403 forbidden` |
| `POST /overlay/admin/invites/{code}/revoke` | aucun | `{found:true, revoked:true ou false}` | `403 forbidden`, `404 invite_not_found` |
| `PATCH /overlay/admin/settings` | champs partiels `name`, `defaultMediaTime`, `maxMediaTime`, `displayMediaFull`, `guestsCanViewBoard` | `{updated:true, guild}` | `400 invalid_payload`, `400 invalid_name`, `403 forbidden` |

`GET /overlay/guild` renvoie `standalone: false`, `name: null` et des réglages `null` si la guilde n'a pas de ligne. `permissions` contient exactement `viewBoard`, `viewQueue`, `play`, `stop`, `addMeme`, `editAnyMeme`, `moderateQueue`, `manageMembers`, `manageInvites`, `manageSettings`.

`PATCH /overlay/profile` exige un membre rattaché. Un pseudo vide après normalisation donne `invalid_display_name` ; un avatar non `http(s)` et non `data:image/{png,jpeg,webp,gif};base64,` est ignoré (`None`) plutôt que refusé. Le nom et l'avatar sont recopiés sur les overlays actifs du membre (`update_author_by_member`). `POST /overlay/leave` sans membre révoque seulement l'overlay courant et ferme sa session avec `4402` (`{left:true}`). Avec un membre, il applique la règle du dernier admin, marque `removed_at`, révoque les overlays actifs du membre et ferme leurs sessions WebSocket avec `4402` (`close_code::REVOKED`).

`GET /overlay/queue` lit au plus 200 jobs (`QUEUE_LIMIT`) et trie `PLAYING` d'abord, puis `priority DESC`, `submission_date ASC`, `id ASC` ; `playing` est le premier job en lecture ou `null`, `pending` contient les autres, `serverTime` est une date ISO 8601 UTC en millisecondes. `DELETE /overlay/queue/{jobId}` passe un job `PENDING` de la guilde à `FAILED` avec `finished_at` ; un identifiant invalide, un job d'une autre guilde ou un job `PLAYING` renvoie `{removed:false}`. `DELETE /overlay/queue` passe tous les `PENDING` à `FAILED` et renvoie le nombre modifié. Le job en cours n'est pas retiré.

Sur le board, `limit` vaut 50 par défaut et est borné à `1..=200` ; `offset` vaut 0 et n'est jamais négatif ; `q` filtre sur le titre, le message, le nom d'auteur et l'URL source. Chaque élément de la liste contient `id`, `guildId`, `mediaAssetId`, `title`, `message`, `createdByName`, `createdAt`, `media`, plus `createdByMemberId` et `canEdit` ; `media` contient `id`, `kind`, `mime`, `durationSec`, `width`, `height`, `isVertical`, `sizeBytes`, `sourceUrl`.

Sur une session invitée, `POST`, `PATCH` et `DELETE` du board répondent `403 {"error":"read_only_session"}` avant toute validation du corps ; la garde s'applique aussi au repli `?token=`. `POST` ingère l'URL via `MediaService` ; si le couple `(guilde, média)` est déjà épinglé, la réponse est `{created:false, item}`. Le quota `MEDIA_BOARD_MAX_TOTAL_MB` produit `422` avec `code: "BOARD_STORAGE_LIMIT_REACHED"`, et les erreurs du pipeline média `422 {"error":"media_ingestion_failed","code":"…","message":"…"}`. Un identifiant invalide ou d'une autre guilde donne `404 item_not_found` en `PATCH` et `{deleted:false}` en `DELETE`.

Les routes admin sont bornées à la guilde du jeton : un membre d'une autre guilde est `404 member_not_found`, une invitation d'une autre guilde `404 invite_not_found`. `GET /overlay/admin/members` ajoute `connected` (au moins un overlay connecté), `deviceCount` (overlays actifs), `lastSeenAt` (le plus récent) et `isSelf`. Les invitations listées excluent les codes appareil et exposent `code`, `role`, `memberId`, `maxUses`, `useCount`, `createdByMemberId`, `expiresAt`, `revokedAt`, `createdAt`, `isActive`.

`POST /overlay/admin/invites` accepte `expiresInHours` entier de 1 à 8760 ou `null`, et `maxUses` entier de 1 à 1000 ou `null` (`null` = jamais / illimité). Le code est tiré dans `ABCDEFGHJKLMNPQRSTUVWXYZ23456789` sur 8 caractères, avec jusqu'à 5 tentatives, après vérification qu'il n'existe pas déjà dans `pairing_codes`. `PATCH /overlay/admin/members/{memberId}` aligne `overlay_clients.session_mode` sur `role.session_mode()` si le rôle change, puis ferme les sessions actives du membre avec `1001` (`close_code::GOING_AWAY`) pour qu'elles se reconnectent ; un changement vers le même rôle ne ferme rien.

`PATCH /overlay/admin/settings` normalise `name` avec `normalize_label` (espaces compactés, 60 caractères au plus, `GUILD_NAME_MAX_CHARS`) et refuse un nom vide. Pour les durées : champ absent = conservé ; `null` ou `<= 0` = effacé ; entier de 1 à 86400 = nouvelle valeur ; autre valeur = `400 invalid_payload`. Un corps sans aucun champ exploitable vaut `invalid_payload`. Les réglages de lecture modifiés sont rediffusés aux overlays connectés (`broadcast_settings`).

## Routes du panneau serveur

Les routes de cette section exigent `Authorization: Bearer <ADMIN_UI_TOKEN>` (`apps/lc-server/src/http/admin/auth.rs`). Réponses d'accès : `503 admin_ui_disabled` si le jeton admin est vide, `403 admin_local_only` si `ADMIN_UI_LOCAL_ONLY` est actif et le client non loopback, `401 unauthorized` si le jeton est absent ou faux ; une mutation sans origine valide reçoit `403 invalid_origin`.

### `POST /admin/api/guilds`

Corps : `{"name": "Les Sardines"}`.

Le nom est trimé, ses espaces compactés et sa longueur bornée à 60 caractères (`GUILD_NAME_MAX_CHARS`, même normalisation que ci-dessus). Un nom absent ou vide donne `400 {"error":"invalid_name"}`. Le handler génère un `GuildId` (jusqu'à 5 essais), appelle `create_standalone`, puis émet un code admin : rôle `ADMIN`, `max_uses = 1`, expiration à 7 jours (`ADMIN_CODE_TTL_HOURS`), `created_by_member_id = null`. Réponse :

```json
{
  "created": true,
  "guild": { "id": "9123456789012345678", "name": "Les Sardines" },
  "adminInvite": { "code": "K7QXM2PD", "role": "admin", "memberId": null, "maxUses": 1, "useCount": 0, "createdByMemberId": null, "expiresAt": "2026-11-17T12:00:00.000Z", "revokedAt": null, "createdAt": "2026-11-10T12:00:00.000Z", "isActive": true }
}
```

Un échec base donne `500 internal_error`. La route ne crée qu'une guilde indépendante ; elle ne crée pas de membre.

### `POST /admin/api/guilds/{guild_id}/admin-code`

Aucun corps. Le chemin est validé par `GuildId::parse` ; un identifiant invalide donne `400 {"error":"invalid_guild_id"}`. Le handler émet un nouveau code admin à usage unique, valable 7 jours :

```json
{
  "created": true,
  "guildId": "9123456789012345678",
  "adminInvite": { "code": "M4PZK9RT", "role": "admin", "memberId": null, "maxUses": 1, "useCount": 0, "createdByMemberId": null, "expiresAt": "2026-11-17T12:00:00.000Z", "revokedAt": null, "createdAt": "2026-11-10T12:00:00.000Z", "isActive": true }
}
```

Le handler ne vérifie pas que la guilde existe : `issue_admin_code` appelle directement `create_invite`, qui insère une ligne dans `guild_invites`. Cette route sert donc aussi de récupération pour une guilde déjà créée.

### Purge

`POST /admin/api/guilds/{guild_id}/purge` exige `confirmGuildId` égal à l'identifiant de la route (`400 confirm_guild_id_mismatch` sinon) et accepte `removeOrphanMedia` (booléen, `true` par défaut). Après avoir libéré les jobs `PLAYING` et fermé les overlays connectés, la purge supprime les clients overlay, puis les invitations, puis les membres (`delete_by_guild` sur `guild_invites` puis `guild_members`). Cet ordre respecte `overlay_clients.member_id`. La réponse détaille les compteurs dans `deleted` (`guildSettings`, `overlayClients`, `ingestClients`, `pairingCodes`, `playbackJobs`, `memeBoardItems`, `orphanMediaAssets`, `orphanMediaFileDeleteErrors`).

## Overlay multi-guilde

### Adhésions dans `config.json`

Le fichier est `%APPDATA%\LiveChat\config.json` (`apps/lc-overlay/src/config.rs`). Il contient un tableau `memberships` :

```json
{
  "serverUrl": "https://api.exemple.tld",
  "clientToken": "jeton-guilde-active",
  "clientId": "2f1c9e4a-3b7d-4c1e-9a2f-5d6e7f8a9b0c",
  "guildId": "9123456789012345678",
  "guestMode": false,
  "memeBindings": { "Ctrl+Shift+1": "média-1" },
  "memberships": [
    { "serverUrl": "https://api.exemple.tld", "guildId": "9123456789012345678", "guildName": "Les Sardines", "clientToken": "jeton-guilde-active", "clientId": "2f1c9e4a-3b7d-4c1e-9a2f-5d6e7f8a9b0c", "memberId": "6a5b4c3d-2e1f-4a9b-8c7d-6e5f4a3b2c1d", "displayName": "Alice", "role": "admin", "guestMode": false, "muted": false, "memeBindings": { "Ctrl+Shift+1": "média-1" } }
  ]
}
```

Les champs historiques de premier niveau (`serverUrl`, `clientToken`, `clientId`, `guildId`, `guestMode`, `memeBindings`) sont le miroir de la guilde active. Le renderer d'affichage, le tray et les raccourcis continuent de les lire. `authorName` et `deviceName` restent des champs legacy : le miroir ne les recopie pas.

`reconcile_memberships` applique ces règles : les adhésions dont `serverUrl`, `guildId` ou `clientToken` est vide sont supprimées ; les doublons de `guildId` sont supprimés en gardant le premier ; si les champs historiques forment une configuration appairée complète, ils font foi et l'adhésion de la guilde active est mise à jour (`serverUrl`, `clientToken`, `clientId`, `guestMode`, `memeBindings`) ou créée avec `displayName = authorName` ; sinon, la guilde active est considérée comme purgée : son adhésion est retirée si `guildId` est renseigné, la première restante est recopiée dans les champs historiques, ou `clientToken`, `clientId`, `guildId` et `guestMode` sont remis à zéro s'il n'en reste aucune.

`join` recopie une adhésion dans les champs historiques et la rend active. `activate` fait de même pour une adhésion existante. `update_membership` modifie une adhésion et met à jour le miroir si elle est active. `leave` retire l'adhésion ; si elle était active, `clientToken`, `clientId`, `guildId`, `guestMode` et `memeBindings` sont effacés, puis `reconcile_memberships` promeut la suivante. `membership_views` expose les adhésions sans jeton pour l'interface.

### Un superviseur WebSocket par guilde

`ws::start` démarre un superviseur pour chaque adhésion au lancement. `ensure_supervisor` ne crée rien si la guilde en a déjà un ; il ouvre un canal de commandes par guilde et lance `supervise`. `request_reconnect` envoie `Reconnect` aux superviseurs existants et démarre ceux qui manquent ; un superviseur dont l'adhésion a disparu s'arrête de lui-même.

`supervise` relit la configuration à chaque tour : adhésion absente, le superviseur s'arrête ; `config.enabled = false` donne le statut `disabled` avec la raison « overlay désactivé », sans connexion, en attente d'une commande ; `membership.muted = true` donne le statut `disabled` avec « guilde en sourdine », sans connexion ; sinon il construit l'URL `ws(s)://…/ws`, envoie `hello` avec le jeton de l'adhésion et le mode de session, puis un heartbeat toutes les 15 s.

Les statuts sont suivis par guilde (`state.rs`, `NetSnapshot`) et exposés par `guilds_payload`. Les codes de fermeture sont interprétés par `close_step` : `4401` et `4402` retirent l'adhésion ; `4403` marque l'overlay obsolète sans retrait ; `4409` et `4408` réessaient ; `1000` et `1003` s'arrêtent en erreur ; les autres codes et l'absence de code réessaient avec un backoff de 1 s, 2 s, puis 3 s (`4409` : 1 s, 2 s, 4 s, 8 s, 16 s, puis 30 s au plus).

### Sourdine

`guild_set_muted` exige `muted` (booléen) et accepte `guildId` optionnel ; sans `guildId`, la cible est la guilde active. Il met à jour `membership.muted`, appelle `ensure_supervisor` puis envoie `Reconnect`. Une guilde coupée n'ouvre aucune connexion : ses médias ne peuvent plus être reçus ni affichés. Si elle occupait l'écran, la fin de session déclenche `release_guild_playback`, qui libère l'écran.

### Arbitrage d'affichage par file locale

Un seul média occupe l'écran, toutes guildes confondues. `AppState` conserve le couple `(guild_id, job_id)` affiché et une `VecDeque<QueuedPlay>` (`apps/lc-overlay/src/state.rs`).

- `AppEffects::present_play` : si une autre guilde occupe l'écran, le `play` est mis en file (`enqueue_play`, dédoublonné par `job_id`) ; sinon il devient la lecture courante et est livré au renderer.
- `on_stop` : retire de la file les `play` de la guilde (le job ciblé si `jobId` est fourni, sinon tous), puis, si une autre guilde occupe l'écran, s'arrête là ; sinon libère l'écran, délivre `stop` et appelle `advance_local_queue`.
- `on_welcome` avec `current_job` : `adopt_current_job` ajoute le temps déjà écoulé à `start_offset_sec`, puis `present_play` applique le même arbitrage.
- `release_guild_playback` : à la fin d'une session, retire les `play` en attente de la guilde et, si elle occupait l'écran, libère celui-ci, émet un arrêt `disconnected` et affiche le suivant.
- `advance_local_queue` : si rien n'est à l'écran, dépile le prochain `play` et l'affiche.

La file locale est FIFO. Aucune commande ne la réordonne.

### Retrait d'une adhésion sur fermeture 4401 ou 4402

Sur `4401` (jeton invalide) ou `4402` (appareil révoqué), `close_step` renvoie une étape terminale avec `purge: true`. `supervise` appelle `purge_pairing` : `config.leave(guild_id)` retire l'adhésion et promeut la suivante si nécessaire ; les raccourcis sont réappliqués (`shortcuts::reapply_for_session`) ; les settings sont émis vers le renderer ; le superviseur s'arrête ; si plus aucune adhésion ne reste, la fenêtre d'appairage est affichée. Les codes `4403`, `1000`, `1003` et les erreurs de connexion ne retirent pas l'adhésion.

### Commandes Tauri et fenêtres autorisées

`COMMAND_MATRIX` (`apps/lc-overlay/src/commands.rs`) limite chaque commande à certaines fenêtres ; toute commande non listée pour la fenêtre émettrice est refusée.

| Commande | Charge utile | Effet |
|----------|--------------|-------|
| `guilds_list` | aucune | Renvoie `{activeGuildId, enabled, profile:{displayName,avatar}, guilds:[{membership,connection,statusLabel,playing}]}` ; aucun jeton |
| `guild_select` | `{guildId}` | Rend la guilde active ; `invalid_guild_id` ou `unknown_guild` sinon ; réapplique raccourcis, settings, tray et événement `overlay://guilds` |
| `guild_set_muted` | `{guildId?, muted}` | Met à jour la sourdine ; `invalid_payload` si `muted` manque, `unknown_guild` si inconnue |
| `guild_leave` | `{guildId}` | Appelle `POST /overlay/leave` ; `invalid_guild_id` si absent, `unknown_guild` si inconnue ; sur `409`, conserve l'adhésion et renvoie le code d'erreur ; sur toute autre réponse ou erreur réseau, retire l'adhésion localement |
| `guild_api` | `{guildId?, method?, path?, body?}` | Appel REST authentifié vers `/overlay/…` ; méthodes `GET`, `POST`, `PATCH`, `DELETE` ; renvoie `{ok,status,body}` |
| `guild_media_url` | `{guildId?, assetId}` | URL `/overlay/meme-board/media/{assetId}?token=…` pour `<img>`/`<video>` |
| `pairing_consume` | `{serverUrl, code, deviceName?, displayName?, avatar?}` | Appelle `POST {serverUrl}/overlay/pair/consume`, ajoute l'adhésion et la rend active, masque la fenêtre d'appairage, affiche l'overlay si activé, reconnecte les superviseurs |

`guild_leave` et `guild_api` ciblent par défaut la guilde active. `guild_api` impose que le chemin commence par `/overlay/` et ne contienne ni `..` ni `\` ; le jeton est ajouté par le shell et n'est jamais exposé au board. `overlay_get_config` retire `clientToken` des projections des fenêtres `overlay` et `pairing` ; seule la fenêtre `board` le reçoit.

| Fenêtre | Commandes autorisées |
|---------|----------------------|
| `overlay` | `overlay_get_config`, `overlay_renderer_ready`, `overlay_report_error`, `overlay_report_playback_state`, `overlay_report_playback_stop` |
| `board` | `overlay_get_config`, `overlay_report_error`, `meme_board_get_bindings`, `meme_board_set_bindings`, `meme_board_trigger`, `meme_board_stop`, `overlay_check_updates`, `overlay_install_update`, `pairing_consume`, `guilds_list`, `guild_select`, `guild_set_muted`, `guild_leave`, `guild_api`, `guild_media_url` |
| `pairing` | `overlay_get_config`, `overlay_report_error`, `pairing_consume` |

Toute autre fenêtre, y compris un libellé vide ou inconnu, n'a aucune commande autorisée.

## Liaison Discord

Une guilde indépendante peut être rattachée à un serveur Discord sans changer
son identifiant local :

1. un administrateur d'overlay (`manageSettings`) demande un code avec
   `POST /overlay/admin/discord-link-code`, qui répond
   `{"code": "<CODE>", "expiresAt": "<iso ms>", "commands": ["/lier", "/link"]}`.
   Le code suit le format des invitations, expire au bout de 10 minutes et est à
   usage unique ; chaque nouvelle demande supprime les codes non utilisés de la
   guilde (`DiscordLinkCodesRepository::replace_for_guild`) ;
2. un administrateur du serveur Discord saisit ce code avec la commande slash
   `/lier` (français) ou `/link` (anglais). Seuls les administrateurs Discord
   peuvent l'exécuter et toutes ses réponses sont éphémères ;
3. `DiscordLinkCodesRepository::redeem` pose `guilds.discord_guild_id` et marque
   le code utilisé. Un serveur déjà lié à une autre guilde est refusé
   (`AlreadyLinkedElsewhere`, le code reste utilisable) ; lier la guilde à un
   autre serveur remplace la liaison précédente ;
4. `DELETE /overlay/admin/discord-link` efface la liaison (`{"linked": false}`).
   Les deux routes répondent `409 {"error":"guild_not_standalone"}` pour une
   guilde native.

| Méthode | Chemin | Droit | Réponse |
|---------|--------|-------|---------|
| `POST` | `/overlay/admin/discord-link-code` | `manageSettings` | `200` avec code, `expiresAt` et commandes ; `409 guild_not_standalone` |
| `DELETE` | `/overlay/admin/discord-link` | `manageSettings` | `200 {"linked": false}` ; `409 guild_not_standalone` |

Résolution : pour **toutes** les commandes sauf `/lier`, le bot remplace le
snowflake du serveur par `GuildsRepository::resolve_discord(snowflake)`. Un
serveur lié vise donc la guilde indépendante pour ses réglages, sa file, son
meme board et ses overlays. La ligne `guilds` native du serveur Discord (si elle
existait déjà) est laissée telle quelle : `guild_create` ne la crée plus pour un
serveur lié, ne la supprime pas, et ses overlays cessent simplement de recevoir
les commandes de ce serveur, désormais destinées à la guilde indépendante.

## Limites connues

- Aucun message WebSocket ne transporte la file d'attente : l'interface interroge `GET /overlay/queue`.
- La file d'attente n'a pas de réordonnancement : les routes se limitent à la lecture et à la suppression.
- Un job retiré de la file passe à l'état `FAILED` ; la route ne retire pas le job `PLAYING`.
- Un changement de rôle ferme les sessions WebSocket du membre avec le code `1001` pour qu'elles se reconnectent et reprennent leur mode à jour.
- Les raccourcis clavier ne valent que pour la guilde active : ils sont lus dans `config.meme_bindings`, miroir de cette guilde.
- L'URL des médias du board embarque encore le jeton en query (`board_media_url`), ce que le serveur accepte uniquement sous `/overlay/meme-board/*`.
- L'arbitrage local d'affichage n'a pas été validé contre le délai de surveillance du moteur de lecture côté serveur. Le moteur libère un job `PLAYING` au-delà de sa durée attendue plus `stale_release_grace_ms` (10 s par défaut, `crates/lc-playback/src/lib.rs`) ; l'attente d'un `play` dans la file locale n'est pas couverte par un test d'interaction.
