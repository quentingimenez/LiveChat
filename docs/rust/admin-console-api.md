# Console d'administration — contrat d'API (v2)

Référence commune du back (`apps/lc-server/src/http/admin/`) et du front
(`apps/lc-admin-web/`). La maquette visuelle est `docs/mockups/admin-panel/`.
Ce document remplace le contrat du panneau 7b pour tout ce qu'il décrit ;
pas de compatibilité avec l'ancien panneau `static/admin/` (il est supprimé).

Conventions communes :

- JSON en camelCase, dates ISO 8601 UTC avec millisecondes, tailles en octets.
- Erreur : statut HTTP + `{"error": "<code_snake_case>"}` (+ `message` facultatif).
- Toutes les routes `/admin/api/**` exigent une session (voir § 1) ; les
  mutations exigent en plus une origine vérifiée (`Origin` / `Referer`, règle
  AUD-COR-14 existante), sinon `403 invalid_origin`.
- Les suppressions irréversibles prennent `{"confirmGuildId": "<id>"}` dans le
  corps (règle existante `require_guild_confirmation`).
- Une route « transposée » renvoie **exactement** le JSON de la route overlay
  citée ; la logique est factorisée dans une fonction partagée, pas copiée.

## 1. Authentification par session

Plus de jeton `Authorization: Bearer` ni de `localStorage`. Une session est un
cookie `lc_admin_session` : `HttpOnly; SameSite=Strict; Path=/admin;
Max-Age=604800` (+ `Secure` si `API_URL` est en `https`). Valeur : 32 octets
aléatoires en base64url ; la base ne stocke que leur SHA-256.

Table `admin_sessions` (migration `0010_admin_sessions.sql`) :
`token_hash TEXT PRIMARY KEY, method TEXT NOT NULL ('discord'|'secret'),
discord_user_id TEXT, display_name TEXT, avatar_url TEXT,
created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
last_seen_at INTEGER NOT NULL` (millisecondes). Deux bornes indépendantes :
**expiration absolue à 7 jours** (`expires_at`) et **inactivité de 24 heures**
(`last_seen_at`). Chaque requête authentifiée rafraîchit `last_seen_at` et
repousse donc la fenêtre d'inactivité ; une session inactive depuis 24 h est
refusée à la prochaine requête (`401`, reconnexion nécessaire), même si le
cookie est encore valide : le serveur reste l'autorité. Le cookie garde
`Max-Age=604800` (7 jours) comme borne extérieure ; le navigateur le conserve
donc jusqu'à 7 jours, mais le serveur refuse toute session après 24 h sans
activité — et le `401` renvoie `Set-Cookie: lc_admin_session=; Max-Age=0` pour
que le navigateur cesse d'envoyer un cookie mort (aucun `Set-Cookie` si la
requête n'en portait pas). Les lignes expirées **ou inactives** sont supprimées
à chaque lecture et au démarrage.

Variables d'environnement (dans `Config`) :

| Variable | Rôle |
|---|---|
| `ADMIN_UI_TOKEN` (existante) | secret d'administration de secours ; vide = connexion par secret désactivée ; non vide : au moins 32 caractères, 8 caractères distincts, sans motif répété (sinon refus au démarrage — changement cassant ; générer avec `openssl rand -base64 32`) |
| `DISCORD_CLIENT_ID` (existante) | application OAuth2 |
| `DISCORD_CLIENT_SECRET` (nouvelle, secret) | échange du code OAuth2 |
| `ADMIN_DISCORD_USER_IDS` (nouvelle) | identifiants Discord autorisés, séparés par des virgules |

La connexion Discord est disponible si les trois dernières sont renseignées.
Si aucune méthode n'est disponible : toutes les routes `/admin/api/**` et
`/admin/auth/*` (sauf `GET /admin/auth/session`) répondent
`503 admin_ui_disabled`. Les contrôles `ADMIN_UI_LOCAL_ONLY` et
`ADMIN_TRUST_FORWARDED_FOR` restent appliqués à toutes ces routes : avec le
second actif et un pair déclaré dans `TRUSTED_PROXIES`, la chaîne
`X-Forwarded-For` est parcourue de droite à gauche en sautant les proxys
déclarés (`X-Real-IP` en repli quand la chaîne est absente ou sans saut non
vide).

| Route | Effet |
|---|---|
| `GET /admin/auth/session` | `200 {"method":"discord"\|"secret","user":{"id","name","avatarUrl"}\|null,"expiresAt"}` ; sans session `401 {"error":"unauthorized","methods":{"discord":bool,"secret":bool}}` |
| `POST /admin/auth/secret` `{"secret"}` | comparaison à temps constant ; `204` + `Set-Cookie` ; mauvais secret `401 invalid_secret` ; désactivé `404 method_disabled` |
| `GET /admin/auth/discord` | `302` vers `https://discord.com/oauth2/authorize` (scope `identify`, `state` posé dans le cookie `lc_admin_oauth_state`, `HttpOnly; SameSite=Lax; Path=/admin/auth; Max-Age=600`) ; le `state` déjà épinglé dans le navigateur est réutilisé et le cookie réémis (fenêtre de 10 minutes rafraîchie), un cookie absent ou d'une autre forme reçoit un `state` neuf ; `redirect_uri` = `{API_URL}/admin/auth/discord/callback` |
| `GET /admin/auth/discord/callback?code&state` | vérifie `state`, échange le code, lit `GET /users/@me`, refuse un id hors liste ; succès `302 /admin/#/dashboard` + cookie ; échec `302 /admin/#/login?error=<code>` (`oauth_state`, `oauth_failed`, `not_allowed`) |
| `POST /admin/auth/logout` | supprime la session, efface le cookie, `204` |

L'échange OAuth2 passe par un trait injectable (`AdminOAuth`) dans `AppState`,
avec une implémentation `reqwest` et une fausse implémentation pour les tests.

Limitation de débit (`http/rate_limit.rs`) : seules les routes de connexion
(`/admin/auth/secret`, `/admin/auth/discord*`) sont dans le budget strict des
secrets devinables (100 requêtes / 15 min / IP, partagé avec la consommation des
codes d'appairage). La page, les assets, `/admin/api/**`, le flux
`/admin/api/events`, `/admin/auth/session` et `/admin/auth/logout` sont dans le
budget général (1000 / 15 min) : ils exigent le cookie de session, et une visite
de la console charge assez de ressources pour épuiser le budget strict en
quelques écrans.

## 2. Assets

La console est une application Preact + TypeScript construite par Vite dans
`apps/lc-admin-web/dist/`, embarquée dans le binaire avec `rust-embed`.
`GET /admin` → `308 /admin/` ; `GET /admin/` → `index.html` ; les fichiers de
`dist/assets/` sont servis avec leur type MIME. Si `dist/` est absent à la
compilation, `/admin/` sert une page minimale « console non compilée ».
CSP : `style-src 'self'` (plus de `'unsafe-inline'`).

## 3. Lecture

| Route | Contenu |
|---|---|
| `GET /admin/api/overview` | existant, inchangé (tableau de bord, listes) |
| `GET /admin/api/guilds/{id}` | une entrée de `overview.guilds[]` enrichie : `guestsCanViewBoard`, `boardQuotaBytes` (`null` = quota du serveur), `discord.botPresent` (`bool\|null`), `members` (= `GET /overlay/admin/members`), `invites` (= `GET /overlay/admin/invites`), `bans` (= `GET /overlay/admin/bans`), `queue` (= `GET /overlay/queue`), `overlays.clients[]` enrichis de `memberId`, `role`, `overlayVersion`, `platform`, `guildSettings` (§ 6) ; `404 guild_not_found` |
| `GET /admin/api/guilds/{id}/media-board/items` | = `GET /overlay/media-board/items` |
| `GET /admin/api/health` | `{"status":"ok"\|"degraded","checks":[{"id":"database"\|"discord"\|"ffmpeg"\|"ytdlp"\|"piper"\|"ingestions","status":"ok"\|"warn"\|"error","detail":string,"version":string\|null}],"uptimeSec","serverVersion"}` |
| `GET /admin/api/ingestions?limit=50` | `{"items":[{"id","guildId","guildName","source","url","status":"pending"\|"running"\|"done"\|"failed","error":string\|null,"startedAt","finishedAt","durationMs"}]}` (registre en mémoire `IngestionRegistry` + historique gardé en mémoire, 200 dernières) |
| `GET /admin/api/audit?query=&before=&limit=100` | `{"items":[{"id","at","actor":{"method","name"},"action","method","path","status","guildId":string\|null,"clientIp":string\|null}],"nextBefore":string\|null}` |
| `GET /admin/api/audit/export` | même contenu en CSV (`text/csv`, pièce jointe) |
| `GET /admin/api/rights` | § 5 |
| `GET /admin/api/runtime-settings` | existant, étendu § 7 |

`clientIp` est l'adresse cliente résolue : le pair direct quand il n'est pas
déclaré, sinon le premier saut non-proxy de `X-Forwarded-For` (droite → gauche,
proxys déclarés sautés ; `X-Real-IP` en repli). `null` signifie que le pair
direct est un proxy déclaré mais que la chaîne ne résout aucun client : l'IP du
proxy n'est jamais journalisée comme adresse cliente.

## 4. Mutations

Transposées de l'overlay (même corps, même réponse, mêmes codes d'erreur) :

| Route admin | Route overlay d'origine |
|---|---|
| `PATCH /admin/api/guilds/{id}/members/{memberId}` | `PATCH /overlay/admin/members/{id}` |
| `DELETE /admin/api/guilds/{id}/members/{memberId}` | `DELETE /overlay/admin/members/{id}` |
| `POST /admin/api/guilds/{id}/members/{memberId}/ban` | `POST /overlay/admin/members/{id}/ban` (`bannedBy` = `null` : la console) |
| `DELETE /admin/api/guilds/{id}/bans/{banId}` | `DELETE /overlay/admin/bans/{id}` |
| `POST /admin/api/guilds/{id}/invites` | `POST /overlay/admin/invites` |
| `POST /admin/api/guilds/{id}/invites/{code}/revoke` | `POST /overlay/admin/invites/{code}/revoke` |
| `POST /admin/api/guilds/{id}/discord-link-code` | `POST /overlay/admin/discord-link-code` |
| `POST /admin/api/guilds/{id}/device-code` | `POST /overlay/device-code` (corps `{"memberId"}` en plus : l'appareil est rattaché à ce membre) |
| `DELETE /admin/api/guilds/{id}/queue` | `DELETE /overlay/queue` |
| `DELETE /admin/api/guilds/{id}/queue/{jobId}` | `DELETE /overlay/queue/{jobId}` |
| `POST /admin/api/guilds/{id}/queue/reorder` | `POST /overlay/queue/reorder` (`{"jobIds":[…]}` complet → `{"reordered":true,"count":n}` ; `409 stale_queue` si la file a changé, `400 invalid_job_id`/`duplicate_job_id`) |
| `PATCH /admin/api/guilds/{id}/media-board/items/{itemId}` | sans `recipe` : mise à jour de l'item de l'overlay (`title`, `message`, `artworkOnOverlay`) → `{"updated","item"}` ; avec `recipe`/`replace` : éditeur de `media_edit.rs` (découpe : `{"replace":true,"recipe":{"segments":[[début,fin]]}}`) → `202 {"ingestionId"}`. Pas de favori : il est propre à chaque membre et la console n'en est pas un |
| `DELETE /admin/api/guilds/{id}/media-board/items/{itemId}` | `DELETE /overlay/media-board/items/{id}` |

Nouvelles ou étendues :

| Route | Corps → réponse |
|---|---|
| `PATCH /admin/api/guilds/{id}/settings` (étendue) | ajoute `name`, `guestsCanViewBoard`, `boardQuotaMb` (`null` = quota du serveur) aux champs existants ; renvoie l'entrée de `GET /admin/api/guilds/{id}` |
| `POST /admin/api/guilds/{id}/discord-leave` | `{"confirmGuildId"}` → `{"discordLeave":{…}}` (même objet que `purge`) ; ni purge ni déliaison |
| `PATCH /admin/api/overlay-clients/{id}` | `{"label"?, "sessionMode"?: "normal"\|"invite_read_only"}` → client mis à jour ; le socket ouvert reçoit `session.changed` (§ 6) |
| `PATCH /admin/api/overlay-clients/{id}/guild-settings` | `{"volume"?: 0..1, "mediaMuted"?: bool, "paused"?: bool}` (`paused` = `muted` de guilde de l'overlay) → `202 {"delivered": bool}` ; message `device.settings` (§ 6) |
| `PUT /admin/api/rights` | § 5 |
| `POST /admin/api/media/cache/purge` | → `{"removedFiles","freedBytes"}` (cache hors board) |
| `POST /admin/api/media/orphans/clean` | → `{"removedFiles","freedBytes"}` |
| `PATCH /admin/api/runtime-settings` (étendue) | § 7 |

Le quota de board par guilde : colonne `guilds.board_quota_mb INTEGER NULL`
(migration `0011_guild_board_quota.sql`), appliquée à l'ajout d'un média à la
place de `MEDIA_BOARD_MAX_TOTAL_MB` quand elle est renseignée.

## 5. Rôles et droits par surface

Table `role_rights (surface TEXT, action TEXT, role TEXT, allowed INTEGER,
PRIMARY KEY (surface, action, role))` (migration `0012_role_rights.sql`), plus
le drapeau `discordAdminIsAdmin` (table `server_flags(key, value)` dans la
même migration). Une ligne absente = valeur par défaut.

Surfaces, actions et colonnes :

- `overlay` (colonnes `guest`, `member`, `admin`) : une action par variante de
  `lc_domain::Permission` (`receiveMedia`, `viewQueue`, `viewBoard`,
  `triggerPlayback`, `stopPlayback`, `addMedia`, `editOwnMedia`,
  `editAnyMedia`, `moderateQueue`, `manageMembers`, `manageInvites`,
  `manageSettings`, `purgeGuild`). Défauts = `GuildRole::can`.
- `extension` (colonnes `guest`, `member`, `admin`) : `pair` (appairer une
  extension, défaut membre et admin) et `send` (envoyer depuis l'extension,
  défaut membre et admin).
- `discord` (colonnes `none` = compte non lié, `guest`, `member`, `admin`) :
  `msg`, `tts`, `mediaAdd`, `stop`, `overlays`, `pairCode`, `info`. Défauts =
  `lc_discord::rights::requirement`. `/lier` reste réservé aux administrateurs
  Discord et n'est pas réglable.

Cellules figées (renvoyées avec `"fixed": true`, refusées en écriture avec
`400 fixed_right`) : toute la colonne `admin` de `overlay` et `extension`,
`receiveMedia` pour tous, `discord.info` pour tous.

`GET /admin/api/rights` →
```json
{"surfaces":[{"id":"overlay","columns":["guest","member","admin"],
  "actions":[{"id":"addMedia","cells":{"guest":{"allowed":false,"fixed":false,"default":false}, "member":{…}, "admin":{…}}}]}],
 "discordAdminIsAdmin":true}
```
`PUT /admin/api/rights` `{"cells":[{"surface","action","role","allowed"}],
"discordAdminIsAdmin"?: bool}` → même réponse que le `GET`. Après écriture :
le cache des droits est rechargé et les overlays connectés reçoivent
`rights.changed` (§ 6).

Application : un service `RightsStore` (cache en mémoire, `Arc<RwLock<…>>`
dans `AppState`) remplace les appels directs à `GuildRole::can` côté serveur
(`require` de l'overlay), à `requirement` côté bot, et est consulté par
`POST /ingest` qui relit à chaque envoi le rôle du membre auteur du client
d'ingestion (membre exclu ou action refusée : `403 forbidden_role`).

## 6. Messages WebSocket ajoutés (protocole overlay)

- client → serveur, `hello` : champ ajouté `platform` (`windows`|`macos`|
  `linux`) ; `appVersion` (existant) et `platform` sont stockés dans
  `overlay_clients` (`overlay_version`, `platform`, migration
  `0013_overlay_client_info.sql`) et exposés en `overlayVersion` / `platform`.
- client → serveur, `device.settings.state` `{volume, mediaMuted, paused}` :
  envoyé à la connexion et à chaque changement local ; le serveur garde le
  dernier état en mémoire par client (exposé dans `guildSettings`).
- serveur → client, `device.settings` `{volume?, mediaMuted?, paused?}` :
  l'overlay applique et renvoie `device.settings.state`.
- serveur → client, `session.changed` `{sessionMode}` : l'overlay recharge ses
  droits (lecture seule ou non) sans se reconnecter.
- serveur → client, `rights.changed` `{}` : l'overlay relit ses droits
  effectifs (`GET /overlay/guild`).

## 7. Paramètres du serveur à chaud

`admin_runtime.json` (existant, cookies TikTok et YouTube) gagne : `defaultDuration`,
`mediaCacheTtlHours`, `mediaCacheMaxTotalMb`, `mediaBoardMaxTotalMb`,
`mediaMaxSizeMb`, `mediaVideoMaxHeight`, `minOverlayVersion`,
`ttsEnabled`. Valeur absente = valeur d'environnement. `GET` renvoie
`{"hasTikTokCookie":bool,"hasYouTubeCookie":bool,"settings":{clé:{"value","source":"runtime"|"env","default"}}, "readOnly":{…}}`
où `readOnly` liste les réglages d'environnement non modifiables (port,
`API_URL`, chemins, présence des secrets en booléen — jamais leur valeur).
`PATCH` accepte un sous-ensemble des clés modifiables (`null` = revenir à
l'environnement) ; `tiktokCookie` et `youtubeCookie` sont écrits en clair dans
`admin_runtime.json` mais jamais renvoyés par l'API.

## 8. Flux temps réel

`GET /admin/api/events` : Server-Sent Events, session obligatoire. Événements
`overview` (même JSON que `GET /admin/api/overview`, émis à la connexion puis
au plus une fois par seconde quand une guilde, un overlay, la file ou le board
change), `audit` (une entrée du journal), `ingestion` (une entrée de
`/ingestions`). Le front remplace le rafraîchissement toutes les 2 s par ce
flux et ne relit les détails qu'à réception d'un `overview`.
