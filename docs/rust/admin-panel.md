# Panel admin — extraction statique et inventaire API (Phase 7a)

> Statut : **Phase 7a terminée** (extraction + documentation, sans build Rust).
> Périmètre écrit : `static/admin/**` et ce document uniquement. Legacy `server/` non modifié, aucun commit.

## 1. Provenance

| Élément | Référence |
|---|---|
| Source unique (legacy) | `server/src/components/admin/adminRoutes.ts` |
| Fonction génératrice du panel | `buildAdminPanelHtml()`, lignes **639-2243** (template literal pur) |
| Bloc CSS | lignes **646-1014** (`<style>` … `</style>`) |
| Bloc HTML (body) | lignes **1016-1094** |
| Bloc JS | lignes **1096-2240** (`<script>` … `</script>`) |
| Handlers Fastify | lignes **2245-3086** (`AdminRoutes`) |
| Inventaire de référence | `docs/rust/migration-plan.md` annexe A (ligne 139), 26 routes v1 / 25 v2 (ligne 141) |
| Contexte CSP / origine overlay | `docs/rust/migration-plan.md` §12.4 (ligne 190) |
| Cible d'architecture | `docs/rust/migration-plan.md` §7 (ligne 101) et annexe D (ligne 157) |

**Écart de comptage constaté** : le plan estime `adminRoutes` à **2 646 lignes** (annexe S9, ligne 34) ; le fichier réel en fait **3 086** (+440). L'inventaire des 14 routes `/admin` reste exact (vérifié sur les handlers 2245-3086).

## 2. Assets livrés

| Fichier | Lignes (non vides) | Octets | Rôle |
|---|---|---|---|
| `static/admin/index.html` | 86 | 3 608 | structure de la page (ex-template HTML) |
| `static/admin/styles.css` | 329 | 6 712 | styles extraits du `<style>` legacy |
| `static/admin/app.js` | 1 000 | 34 904 | logique client extraite du `<script>` legacy |

Aucun autre fichier créé ou modifié. Les trois fichiers sont UTF-8 **sans BOM**.

Chargement :

- page servie à `GET /admin/` (index), `GET /admin/styles.css`, `GET /admin/app.js` ;
- `index.html` référence `styles.css` et `app.js` en **relatif** : cela impose que `/admin` redirige (301/308) vers `/admin/`, sinon les chemins relatifs résolvent vers `/styles.css` et `/app.js` (voir §11, point P1) ;
- JS chargé via `<script src="app.js" defer></script>` : exécution après parse du DOM, équivalent au script inline en fin de `<body>` du legacy.

## 3. Méthode d'extraction et modifications

Extraction mécanique du template literal :

1. dé-indentation de **6 espaces** (indentation du template) pour CSS et JS ;
2. déséchapement des séquences du template : `\"` → `"` (`.replace(/"/g, '&quot;')`, ligne 1162 legacy) et `\\'` → `\'` (`l\'ID` de la confirmation de purge, ligne 1885 legacy) ;
3. HTML : conservation à l'identique de l'output legacy (lignes 640-645, 1015-1095, 2241-2242), les blocs `<style>` et `<script>` étant remplacés par `<link>` / `<script src>`.

Le template legacy ne contient **aucune interpolation serveur** `${...}` : vérifié par grep — les 4 occurrences `` ${ `` du fichier sont dans des handlers (`overlay-guild-${guildId}` lignes 590/608, `Guild ${guildId}` ligne 2436, `${label}-${authorSuffix}` ligne 2619), jamais dans le template. Le seul « état serveur » visible dans la page est le token saisi par l'utilisateur (localStorage/query), pas une donnée injectée.

Modifications volontaires, limitées et documentées :

| # | Modification | Raison | Équivalence |
|---|---|---|---|
| M1 | 8 attributs `style="margin-top: 8px|10px"` inline → classes `.mt-8` / `.mt-10` | supprimer les styles inline statiques pour viser une CSP plus stricte | même `margin-top`, aucune autre règle |
| M2 | suppression du handler inline `onerror="this.style.display='none'"` sur l'avatar overlay, remplacé par un listener `error` en capture (`app.js`, avant le listener `focusin`) qui pose `hidden` sur `img.overlay-avatar` | handler inline incompatible avec `script-src 'self'` | avatar cassé masqué, comme avant |
| M3 | `<script>` inline → `app.js` externe `defer` | `script-src 'self'` | exécution après DOM parsé, comme le script en fin de body |
| M4 | commentaires d'en-tête de provenance dans `styles.css` et `app.js`, classes utilitaires commentées | traçabilité | aucun impact runtime |

**Non modifiés** : tous les sélecteurs/valeurs CSS et leur ordre de cascade (hors ajout `.mt-8/.mt-10`), tous les identifiants DOM, libellés, textes, chemins, méthodes HTTP, payloads et comportements JS du legacy.

## 4. Comportement runtime (identique au legacy)

- **Token** : lu d'abord dans `?token=` (puis persisté en `localStorage` sous `livechat_admin_token` et synchronisé dans l'URL via `history.replaceState`), sinon depuis `localStorage`. Toutes les requêtes partent en `Authorization: Bearer <token>`.
- **Boot** : si token absent → message d'aide, aucune requête ; sinon `refreshAll()` puis deux `setInterval` : overview toutes les **2 000 ms**, pairing codes toutes les **12 000 ms**.
- **Pause de l'auto-refresh** : si le focus est dans un contrôle éditable, ou pendant 4 s après `focusin`/`input`/`change`, ou 1,5 s après `focusout` (`blockAutoRefresh`), les rafraîchissements automatiques sont sautés.
- **Pas de framework, pas de dépendance externe** : DOM direct + `innerHTML` avec `escapeHtml()` systématique sur les données.
- Les formulaires sont interceptés (`submit` + `preventDefault`) ; les actions passent par la délégation `click` sur `button[data-action]`.

## 5. Mapping UI → endpoints (14 routes de l'inventaire)

L'inventaire annexe A contient **14 routes** : la page `GET /` et 13 routes JSON. Le JS appelle exactement **les 13 routes JSON** (14 sites d'appel) ; la page est servie par le service de fichiers statiques. **Aucun endpoint manquant, aucun appel hors inventaire.**

| # | Action UI (event) | Méthode + chemin | Déclencheur / rafraîchissement |
|---|---|---|---|
| 1 | `GET /admin/` (page) | GET | navigation navigateur (statique en v2) |
| 2 | Affichage métriques + cartes guilds | GET `/admin/api/overview` | boot, bouton « Rafraîchir », auto-refresh 2 s, après la plupart des actions |
| 3 | Lire cookie TikTok | GET `/admin/api/runtime-settings` | boot/refreshAll |
| 4 | Enregistrer / vider cookie TikTok | PATCH `/admin/api/runtime-settings` | boutons « Enregistrer TIKTOK_COOKIE » / « Vider » |
| 5 | Liste des auteurs connus | GET `/admin/api/ingest-authors` | refreshAll, après création d'un client ingest |
| 6 | Créer un client ingest | POST `/admin/api/ingest-clients` | bouton « Créer client ingest » |
| 7 | Sauver les réglages d'une guild | PATCH `/admin/api/guilds/:guildId/settings` | submit du formulaire `data-action="save-settings"` |
| 8 | Stop playback d'une guild | POST `/admin/api/guilds/:guildId/stop` | bouton « Stop playback » |
| 9 | Purger une guild | POST `/admin/api/guilds/:guildId/purge` | bouton « Purger guild » (confirmation `window.prompt` + `confirmGuildId`) |
| 10 | Révoquer un overlay client | POST `/admin/api/overlay-clients/:clientId/revoke` | bouton « Révoquer » dans la liste overlay |
| 11 | Révoquer un client ingest | POST `/admin/api/ingest-clients/:clientId/revoke` | bouton « Révoquer » dans la liste ingest |
| 12 | Lister les pairing codes | GET `/admin/api/pairing-codes?guildId&status=all\|active\|expired` | bouton « Charger », auto-refresh 12 s, filtre guild, révocation |
| 13 | Purger les codes expirés/utilisés | DELETE `/admin/api/pairing-codes/expired?guildId` | bouton « Purger expirés » |
| 14 | Révoquer un pairing code | POST `/admin/api/pairing-codes/:code/revoke` | bouton « Révoquer » d'une ligne active |

Actions purement client sans appel HTTP : saisie/enregistrement du token, bouton « Filtrer pairing codes » (positionne le filtre puis GET #12), copie du token ingest dans le presse-papiers.

## 6. Contrats HTTP détaillés

Toutes les routes JSON sont protégées par `assertAdminAccess` (voir §7). Erreurs d'auth : `503 {"error":"admin_ui_disabled"}`, `403 {"error":"admin_local_only"}`, `401 {"error":"unauthorized"}`.

### 6.1 `GET /admin/api/overview`

Réponse 200 :

```jsonc
{
  "generatedAt": "2026-11-10T…Z",          // ISO 8601
  "cache": { "usedBytes": 0, "maxBytes": 0, "remainingBytes": 0, "usedPercent": 0 },
  "totals": { "guildCount": 0, "overlayClientCount": 0, "overlayConnectedCount": 0,
              "ingestClientCount": 0, "pairingCodeCount": 0 },
  "pairingCodes": { "active": 0, "expired": 0, "total": 0 },
  "guilds": [{
    "id": "123456789012345678",
    "name": "Nom guilde",                 // Discord cache/fetch → nom persisté → "Guild {id}"
    "editableName": "",                    // champ legacy renvoyé mais non utilisé par le JS
    "busyUntil": null,                     // ISO ou null
    "settings": {
      "defaultMediaTime": 30,              // effectif (override DB sinon env DEFAULT_DURATION)
      "maxMediaTime": null,
      "displayMediaFull": false,
      "overrides": { "defaultMediaTime": null, "maxMediaTime": null, "displayMediaFull": null }
    },
    "overlays": {
      "total": 0, "connectedCount": 0,
      "clients": [{ "id": "…", "label": "…", "authorName": null, "authorImage": null,
                    "lastSeenAt": "…", "createdAt": "…", "connected": false, "sessionMode": "normal" }]
    },
    "ingest": { "total": 0, "clients": [{ "id": "…", "label": "…", "lastSeenAt": "…", "createdAt": "…" }] },
    "memeBoard": { "itemsCount": 0, "usedBytes": 0, "maxBytes": 0, "remainingBytes": 0, "usedPercent": 0 },
    "playback": { "pending": 0, "playing": 0, "failed": 0, "done": 0 }
  }]
}
```

Calculs legacy à reproduire : guilds = union (`guilds` + overlay_clients + ingest_clients + meme_board_items + playback_jobs + pairing_codes + cache Discord) triée ; `cache.maxBytes = max(1, MEDIA_CACHE_MAX_TOTAL_MB) × 1024²` ; `memeBoard.maxBytes = max(1, MEDIA_BOARD_MAX_TOTAL_MB) × 1024²` ; cache usé = somme `sizeBytes` des `media_assets` `READY` sans `memeBoardItems` ; `connectedCount` = sockets overlay dédupliqués par clientId (`sessionMode` `invite_read_only` prioritaire).

### 6.2 `GET|PATCH /admin/api/runtime-settings`

- GET → 200 `{ "tiktokCookie": "...", "hasTikTokCookie": true }` (le **cookie complet** est renvoyé au client, comme en v1).
- PATCH body `{ "tiktokCookie": "..." }` → 400 `{"error":"invalid_payload"}` si absent/non-string ; 200 `{"updated":true,"tiktokCookie":"...","hasTikTokCookie":true}`. Persistance legacy : écriture dans le fichier `.env` (`persistRuntimeTikTokCookieToEnvFile`).

### 6.3 `GET /admin/api/ingest-authors` → 200

```jsonc
{ "items": [{ "discordUserId": "...", "displayName": "...", "image": null,
              "lastSeenAt": "…|null", "source": "pairing|ingest|mixed" }],
  "count": 0 }
```

Fusion `pairing_codes` + `ingest_clients`, dédupliquée par `createdByDiscordUserId`, `lastSeenAt` = date la plus récente, tri `displayName` puis `discordUserId` (insensible à la casse).

### 6.4 `POST /admin/api/ingest-clients`

Body `{ "guildId": "...", "authorDiscordUserId": "...", "label": "..." }`.

Erreurs 400 : `invalid_guild_id`, `invalid_author_discord_user_id`, `invalid_label`, `author_not_found`.
Succès 200 : `{"created":true,"ingestApiToken":"<token en clair, une seule fois>","client":{"id","guildId","label","defaultAuthorName","defaultAuthorImage","createdByDiscordUserId","createdAt"}}`.
Le label stocké est `"{label}-{displayName|discordUserId}"` (legacy).

### 6.5 `PATCH /admin/api/guilds/:guildId/settings`

Body partiel `{ "defaultMediaTime"?: number|null, "maxMediaTime"?: number|null, "displayMediaFull"?: boolean }`.
Erreurs : 400 `invalid_guild_id` ; 400 `invalid_payload` si aucun champ exploitable ou type invalide.
Succès 200 : `{"updated":true,"guild":{"id","defaultMediaTime","maxMediaTime","displayMediaFull","busyUntil"}}`.
Legacy : `defaultMediaTime`/`maxMediaTime` entiers `> 0`, `≤ 0`/`null`/`""` → `null` ; upsert `guilds` (création si absente).

### 6.6 `POST /admin/api/guilds/:guildId/stop`

400 `invalid_guild_id`. Succès 200 `{"stopped":true,"guildId":"...","releasedCount":0}` (release des jobs `PLAYING` + broadcast `stop`).

### 6.7 `POST /admin/api/guilds/:guildId/purge`

Body `{ "confirmGuildId": "<guildId exact>", "removeOrphanMedia"?: boolean }`.
Erreurs 400 : `invalid_guild_id`, `confirm_guild_id_mismatch`, `invalid_payload`.
Succès 200 :

```jsonc
{ "purged": true, "guildId": "…",
  "stop": { "releasedPlayingJobs": 0, "disconnectedOverlaySockets": 0 },
  "deleted": { "guildSettings": 0, "overlayClients": 0, "ingestClients": 0, "pairingCodes": 0,
               "playbackJobs": 0, "memeBoardItems": 0, "orphanMediaAssets": 0,
               "orphanMediaFileDeleteErrors": 0 } }
```

Ordre legacy : stop manuel → déconnexion sockets de la room `overlay-guild-{guildId}` → suppression ingest clients → suppressions (overlay clients, pairing codes, jobs, board items, guild) → purge des assets orphelins (fichiers `rm` + `deleteMany`).

### 6.8 `POST /admin/api/overlay-clients/:clientId/revoke`

400 `invalid_client_id` ; 404 `overlay_client_not_found` ; 200 `{"found":true,"revoked":true,"clientId","guildId","label"}`. Si révoqué : `disconnectOverlayClient` (v2 : close `4402 revoked`).

### 6.9 `POST /admin/api/ingest-clients/:clientId/revoke`

400 `invalid_client_id` ; 404 `ingest_client_not_found` ; 200 `{"found":true,"revoked":true,"clientId","guildId","label"}` (`revoked` false si déjà révoqué).

### 6.10 `GET /admin/api/pairing-codes`

Query `guildId?`, `status` = `all` (défaut) | `active` | `expired`.
200 `{"items":[{"code","guildId","createdByDiscordUserId","authorName","expiresAt","usedAt","createdAt","isActive"}],"counts":{"total","active","expired"}}`.
`isActive` = non utilisé et (`expiresAt` null ou futur). Filtrage `active`/`expired` appliqué **avant** le calcul des counts.

### 6.11 `DELETE /admin/api/pairing-codes/expired`

Query `guildId?`. 200 `{"deletedCount":0,"guildId":"…|null"}`. Cible : `usedAt != null` **ou** `expiresAt <= now`.

### 6.12 `POST /admin/api/pairing-codes/:code/revoke`

400 `invalid_code` ; 404 `pairing_code_not_found` ; 200 `{"found":true,"revoked":true,"guildId":"…"}`.
Code **upper-casé** avant recherche ; si actif : `usedAt = now`, `expiresAt = now` (sinon `revoked:false`).

## 7. Authentification et mode local-only

Sémantique v1 (`assertAdminAccess`, lignes 197-225) :

1. `ADMIN_UI_TOKEN` vide → **503** `{"error":"admin_ui_disabled","message":"ADMIN_UI_TOKEN is not configured"}` ;
2. `ADMIN_UI_LOCAL_ONLY` vrai et `request.ip` non loopback → **403** `{"error":"admin_local_only"}`. Loopback reconnu : `127.0.0.1`, `::1`, `0:0:0:0:0:0:0:1`, et `::ffff:127.0.0.1` (préfixe déplié) ;
3. token absent ou différent → **401** `{"error":"unauthorized"}`.

Extraction du token (`getRequestToken`) : `Authorization: Bearer <token>` **prioritaire**, sinon query `?token=<token>`. Comparaison à temps constant (`timingSafeEqual` après égalité de longueur).

Le **HTML de la page** était lui aussi protégé en v1 (le handler `GET /` appelle `assertAdminAccess`) : c'est ce qui justifie l'usage de `?token=` dans l'URL pour une navigation navigateur. En v2 statique, la page est un fichier : soit le service de fichiers reproduit la vérification (query token/Bearer), soit la page est servie publiquement (elle ne contient aucune donnée, toutes les API restant protégées) — décision à trancher en Phase 7b (voir §11).

Points de sécurité hérités, à traiter en v2 :

- token en clair dans l'URL (`?token=`) : fuite possible via historique, `Referer`, journaux serveur/proxy ; le JS le réinjecte dans l'URL à chaque lecture ;
- token en `localStorage` : exposable en cas de XSS (atténué par `escapeHtml` systématique et CSP) ;
- `GET /admin/api/runtime-settings` renvoie le cookie TikTok complet (secret) au navigateur ;
- hypothèse cible du plan (§10 ligne 127) : **cookie de session + Bearer** ; recommander cookie `HttpOnly; SameSite=Strict; Secure` (ou `localhost` sans Secure) + jeton CSRF pour les mutations, ou conserver Bearer pur avec page sans token en URL ;
- `ADMIN_UI_LOCAL_ONLY` est un filtre d'IP applicatif : en Rust, s'appuyer sur `ConnectInfo<SocketAddr>` (et non sur des en-têtes non fiables) sauf `TRUST_PROXY_HEADERS=true` explicite.

## 8. CSP et en-têtes recommandés

La page legacy était incompatible avec une CSP stricte : `<script>` inline, un handler inline `onerror`, 8 attributs `style` inline et 2 styles dynamiques générés par `innerHTML` (`width` des barres de progression). La version extraite supprime les trois premiers besoins.

CSP cible (version extraite telle quelle) :

```http
Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' https://cdn.discordapp.com https://media.discordapp.net; connect-src 'self'; font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'
```

- `script-src 'self'` : possible car plus aucun script/handler inline ;
- `style-src 'self' 'unsafe-inline'` : encore nécessaire à cause des 2 `style="width:…%"` produits par `innerHTML` (`renderMetrics`, `renderGuilds`). **Variante strictement `'self'`** : remplacer ces deux rendus par une mise à jour CSSOM (`span.style.width = …`) — la CSSOM n'est pas restreinte par `style-src` ; changement de rendu nul, à faire en Phase 7b ;
- `img-src` : les avatars d'auteurs sont des URL Discord ; hosts exacts à confirmer en production (`cdn.discordapp.com` attendu, `media.discordapp.net` en marge) ;
- `connect-src 'self'` : tous les `fetch` sont same-origin ;
- pas de police externe (`font-family` locales sans `@font-face`).

Si l'on devait servir la page legacy inline (non retenu), il faudrait `script-src 'unsafe-inline'` (script + handler inline, ou `'unsafe-hashes'` avec hash du handler) **et** `style-src 'unsafe-inline'`.

En-têtes recommandés pour `/admin/*` et `/admin/api/*` :

```http
Cache-Control: no-store
X-Content-Type-Options: nosniff
Referrer-Policy: no-referrer
X-Frame-Options: DENY
Permissions-Policy: camera=(), microphone=(), geolocation=()
```

Les assets statiques peuvent avoir un cache court + `ETag` (`no-cache` en dev) ; l'index et les réponses API doivent rester `no-store`.

## 9. Parité, écarts et points d'attention

| # | Constat | Impact / action |
|---|---|---|
| E1 | Aucun endpoint manquant : les 13 routes JSON sont toutes appelées ; la 14e route est la page | parité API OK |
| E2 | Plan S9 estime `adminRoutes` à 2 646 l. ; réel **3 086 l.** | corriger l'estimation si le plan est réutilisé ; inventaire inchangé |
| E3 | Template sans interpolation serveur (vérifié) | extraction statique sûre, aucun rendu serveur à répliquer |
| E4 | Handler inline `onerror` (avatar) unique | supprimé (M2) pour CSP ; comportement identique |
| E5 | 8 styles inline statiques | convertis en classes (M1) ; rendu identique |
| E6 | 2 styles inline dynamiques (largeur des progress bars) | conservés pour fidélité ; `style-src 'unsafe-inline'` ou refactor CSSOM en 7b |
| E7 | `editableName` est renvoyé par l'API mais jamais utilisé par le JS (`const editableGuildName` déclarée, non lue) | dead data legacy : conserver dans le JSON pour parité, ne pas s'appuyer dessus |
| E8 | Règle CSS `.guild-name-form` dans la media query < 640 px sans élément correspondant | dead CSS legacy : conservée à l'identique (parité) |
| E9 | Page HTML protégée par token en v1, fichier statique en v2 | décision 7b (protéger la page ou page publique sans données) |
| E10 | Dates renvoyées en ISO 8601 par Fastify (objets `Date` sérialisés) ; le JS fait `new Date(value)` et affiche `-` si invalide | v2 : conserver ISO 8601 (tolérer aussi epoch ms pour la robustesse) |
| E11 | Auto-refresh agressif : overview toutes les 2 s par onglet ouvert | charge ~43 200 GET/jour/onglet ; acceptable en local ; envisager SSE/WS ou intervalle plus long en 7b (hors parité) |
| E12 | Création ingest : `ingestApiToken` renvoyé en clair une fois, copiable via presse-papiers | à couvrir par tests contrat + audit de logs (ne pas logger le token) |
| E13 | `window.prompt` pour confirmer la purge (bloquant) + validation serveur `confirmGuildId` | sécurité côté serveur correcte ; UX à revoir éventuellement en 7b |
| E14 | Purge : `removeOrphanMedia` par défaut `true` (si absent) | comportement destructif à conserver tel quel, documenté |
| E15 | `GET /admin/` + assets doivent être servis sous `/admin/` avec slash final (chemins relatifs) | P1 en 7b |
| E16 | Le JS appelle les chemins absolus `/admin/api/...` depuis la page `/admin/` | aucun change si montage identique au legacy (préfixe `/admin`) |

## 10. Points pour la Phase 7b (branchement serveur Rust)

1. **Service statique** : monter `static/admin` via `tower-http::ServeDir` (ou handlers dédiés) sur `GET /admin/{*path}` ; `GET /admin` → redirection 301/308 vers `/admin/` (les liens CSS/JS sont relatifs) ; `index.html` servi pour `/admin/` ; `Content-Type` corrects ; `Cache-Control: no-store` pour l'index et l'API ; en-têtes du §8.
2. **Handlers API** : implémenter les 13 routes JSON (§6) dans `apps/lc-server` (ou module admin dédié) en s'appuyant sur `lc-db`/`lc-domain` ; réponses et noms d'erreurs **identiques** (§6) pour que `app.js` reste inchangé ; `serde` en camelCase, dates ISO 8601.
3. **Agrégation overview** : requêtes counts/agrégats groupés (jobs par statut, board bytes, cache READY sans board), union des guilds, tri stable ; éviter 7 requêtes + N appels Discord par poll si possible (cache court), sans changer le contrat.
4. **Auth middleware** : reproduire l'ordre 503 → 403 → 401 (§7) ; Bearer + fallback query pour la page ; décider page publique vs protégée (E9) ; cookie+Bearer conforme au plan §10, CSRF si cookie ; comparer le token à temps constant.
5. **Intégration WS** : registre des sockets overlay par `guildId/clientId` (pour `overlayConnectedCount` et `clients[].connected`) ; `revoke` → close `4402 revoked` (protocole v2) ; `stop` → release jobs + broadcast `stop` ; `purge` → fermeture des sockets de la guilde.
6. **Réglages runtime** : remplacer l'écriture `.env` legacy par la config v2 (fichier/`config`), avec relecture immédiate ; exposer `hasTikTokCookie` ; décider du renvoi en clair du cookie (parité) ou masquage (sécurité) — écart à valider avec le Lead.
7. **Noms de guildes Discord** : le serveur v2 doit démarrer sans Discord obligatoire ; l'overview doit fonctionner en mode dégradé (nom persisté sinon `Guild {id}`), sans changer le schéma de réponse.
8. **CSP/headers** : poser la CSP cible ; option refactor CSSOM pour passer `style-src 'self'`.
9. **CORS** : si le panel est ouvert depuis la WebView overlay (`http(s)://tauri.localhost`, §12.4), compléter CORS `/admin` ; sinon same-origin only.
10. **Mode local-only** : `ADMIN_UI_LOCAL_ONLY` via `ConnectInfo`, pas via en-têtes non fiables ; bind loopback par défaut.
11. **Sécurité opérationnelle** : ne pas logguer tokens/cookie ; `ADMIN_UI_TOKEN` requis au boot (ou 503 explicite) ; rate-limit admin ; audit log des mutations (stop/purge/revoke/créations).
12. **Tests** : tests contrat JSON des 13 routes (fixtures), test statique (fichiers servis, redirection `/admin` → `/admin/`, CSP), et vérifier que `node --check static/admin/app.js` reste vert dans la CI si Node est disponible.
13. **Nettoyage** : `buildAdminPanelHtml()` restera dans `server/` jusqu'au cutover (Phase 8/9) ; ne pas le supprimer en 7b.

## 11. Vérifications effectuées

- `node --check static/admin/app.js` → **exit 0** (syntaxe JS valide).
- Script de contrôle de fidélité (hors dépôt, `%TEMP%\admin_extract_check.js`) comparant les fichiers extraits au template legacy après application des seules modifications M1-M3 :
  - `CSS: PASS` (367 lignes legacy, comparaison exacte caractère par caractère après normalisation CRLF) ;
  - `JS: PASS` (1 143 lignes legacy) ;
  - `HTML: PASS` (91 lignes) ;
  - `EXIT=0`.
- Greps de contrôle : aucune interpolation `${` dans le template legacy (4 occurrences ailleurs, dans les handlers) ; une seule paire d'échappements `\"` / `\\'`, correctement déséchappés ; 8 styles inline statiques convertis ; 1 handler inline `onerror` supprimé.
- Contrôle des fichiers livrés : UTF-8 sans BOM ; **aucun** `<script>` inline, **aucun** `onerror`, **aucun** attribut `style=` dans `index.html` ; les 2 `style="width:…"` restants sont dans `app.js` et documentés (E6).
- Aucun fichier hors `static/admin/` et `docs/rust/admin-panel.md` modifié ; `server/` legacy intact ; aucun commit.

## 12. Références

- `server/src/components/admin/adminRoutes.ts` (source legacy, 3 086 lignes)
- `docs/rust/migration-plan.md` — annexe A (inventaire REST), §7 (planning), §10 (risques/hypothèses), §12.4 (CSP/CORS overlay), §12.8 (verdict POC serveur)
- `docs/rust/protocol-v2.md` — close codes (4402 `revoked`, 1001 `server_shutdown`) pour l'intégration WS de la révocation
- `static/admin/index.html`, `static/admin/styles.css`, `static/admin/app.js`
