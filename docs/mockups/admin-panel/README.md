# Maquette — nouvelle console d'administration du serveur

Proposition de design, **sans backend** : `index.html` s'ouvre par double-clic
(aucun build, aucun CDN, aucun framework). Les données sont fictives et générées
dans `app.js` ; chaque action modifie ces données en mémoire et affiche une
notification. Rien n'est envoyé au serveur.

Identité : palette « régie » du board (`apps/lc-overlay/web/app/styles/base.css`),
ardoise pour les surfaces, **ambre réservé à ce qui est à l'antenne**. Les
actions principales sont en bleu, les actions destructrices en rouge.

## Écrans

| Écran | Adresse | Contenu |
|---|---|---|
| Connexion | `#/login` | « Se connecter avec Discord » + secret d'administration de secours |
| Tableau de bord | `#/dashboard` | santé du serveur, alertes, chiffres clés, guildes à l'antenne, stockage, activité récente |
| Guildes | `#/guilds` | liste dense, recherche (nom ou identifiant), filtres, création d'une guilde indépendante, lien d'invitation du bot |
| Détail d'une guilde | `#/guilds/{id}/{onglet}` | 9 onglets, voir ci-dessous |
| Overlays | `#/devices` | tous les appareils de toutes les guildes, filtres (en ligne, lecture seule, obsolètes) |
| Codes et invitations | `#/codes` | invitations d'overlay et codes d'appairage Discord, purge des expirés |
| Clients d'ingestion | `#/ingest` | extensions de navigateur, rôle de l'auteur (signale ceux qui ne devraient plus envoyer), création (jeton affiché une fois), révocation |
| Rôles et droits | `#/roles` | matrice invité / membre / admin, séparée pour l'overlay, l'extension et le bot Discord ; colonne « Contrôle » qui dit ce que le serveur vérifie vraiment aujourd'hui |
| Médias et stockage | `#/media` | cache, occupation par guilde, disque, ingestions récentes et leurs échecs |
| Journal | `#/activity` | audit des actions d'administration, recherche, export |
| Paramètres du serveur | `#/settings` | accès, Discord, lecture, médias et quotas, sources, overlays, synthèse vocale, réseau |

Onglets du détail d'une guilde : Vue d'ensemble, Membres (rôle, exclusion),
Overlays (état, version, session, **dialogue « Droits »**), Invitations et codes,
Board média (occupation, quota, derniers ajouts), File d'attente (à l'antenne,
retrait, vidage), Réglages (les mêmes que dans l'overlay), Discord (liaison,
présence du bot), Zone de danger.

Dialogue « Droits » d'un overlay : renommer l'appareil, choisir le mode de
session (`normal` / `invite_read_only`), régler à distance le son et la pause
de la guilde sur cet appareil (volume, son des médias coupé, pause), voir en
direct les droits effectifs (les 13 `Permission` de
`crates/lc-domain/src/enums.rs`, tels que réglés dans « Rôles et droits »,
croisés avec le rôle du membre et le réglage « les invités voient le board »),
révoquer.

Page « Rôles et droits » : les valeurs par défaut reprennent `GuildRole::can`
(overlay) et `requirement` de `crates/lc-discord/src/rights.rs` (bot). Les
cases pleines sont des droits toujours accordés (un admin garde tout, tout le
monde reçoit les médias) ; `/lier` reste réservé aux administrateurs Discord.
Côté extension, le rôle n'est vérifié qu'à l'appairage
(`POST /overlay/ingest-code`) : la page le signale au lieu de le masquer.

Les suppressions irréversibles (vider le board, révoquer tous les accès,
supprimer la guilde) demandent de saisir l'identifiant de la guilde, comme le
`confirmGuildId` des routes actuelles.

## Routes : ce qui existe, ce qui manque

### Déjà couvert par `/admin/api` (`apps/lc-server/src/http/admin/`)

| Action de la maquette | Route existante |
|---|---|
| Tableau de bord, liste des guildes, overlays, clients d'ingestion, occupation | `GET /admin/api/overview` |
| Créer une guilde indépendante | `POST /admin/api/guilds` |
| Code admin d'une guilde | `POST /admin/api/guilds/{id}/admin-code` |
| Réglages : durées, plein écran | `PATCH /admin/api/guilds/{id}/settings` |
| Arrêter la lecture | `POST /admin/api/guilds/{id}/stop` |
| Révoquer tous les accès | `POST /admin/api/guilds/{id}/revoke-clients` |
| Vider le board | `POST /admin/api/guilds/{id}/media-board/clear` |
| Supprimer la guilde (+ fichiers orphelins, + départ du bot) | `POST /admin/api/guilds/{id}/purge` |
| Délier Discord (+ départ du bot) | `POST /admin/api/guilds/{id}/discord-unlink` |
| Révoquer un overlay | `POST /admin/api/overlay-clients/{id}/revoke` |
| Créer / révoquer un client d'ingestion | `POST /admin/api/ingest-clients`, `POST /admin/api/ingest-clients/{id}/revoke`, `GET /admin/api/ingest-authors` |
| Codes d'appairage : liste, révocation, purge | `GET /admin/api/pairing-codes`, `POST …/{code}/revoke`, `DELETE …/expired` |
| Cookie TikTok | `GET` / `PATCH /admin/api/runtime-settings` |

### Existe côté overlay seulement (`/overlay/admin/*`, jeton d'un admin de guilde)

La logique est écrite ; il faut l'exposer sous `/admin/api/guilds/{id}/…` avec
l'authentification de la console.

| Action | Route overlay à transposer |
|---|---|
| Lister les membres, changer un rôle, exclure | `GET /overlay/admin/members`, `PATCH` / `DELETE /overlay/admin/members/{id}` |
| Lister, créer, révoquer une invitation | `GET` / `POST /overlay/admin/invites`, `POST …/{code}/revoke` |
| Nom de la guilde, « les invités voient le board » | `PATCH /overlay/admin/settings` (à fusionner dans le `PATCH settings` admin) |
| Code de liaison Discord | `POST /overlay/admin/discord-link-code` |
| File d'attente : lire, retirer, vider | `GET` / `DELETE /overlay/queue`, `DELETE /overlay/queue/{jobId}` |
| Code d'appairage d'un appareil | `POST /overlay/device-code` |
| Board : lister, modifier (titre, découpe, favori), supprimer un média | `GET /overlay/media-board/items`, routes de `media_edit.rs`, `DELETE …/items/{id}` |

### À créer

| Besoin | Proposition |
|---|---|
| Connexion Discord, session, déconnexion | `GET /admin/auth/discord` + `/callback` (OAuth2, scope `identify`), `POST /admin/auth/secret`, `POST /admin/auth/logout`, cookie de session `HttpOnly` ; liste d'identifiants Discord autorisés |
| **Changer le mode de session / le nom d'un overlay** | `PATCH /admin/api/overlay-clients/{id}` `{sessionMode, label}` + notification du socket (aujourd'hui le mode découle du rôle à l'appairage) |
| Détail d'une guilde | `GET /admin/api/guilds/{id}` (l'overview actuel renvoie tout, toutes les 2 s) |
| Version et système de chaque overlay | à remonter dans le `hello` WebSocket et à stocker |
| Faire quitter le bot sans purger ni délier | `POST /admin/api/guilds/{id}/discord-leave` |
| Quota de board propre à une guilde | colonne + champ dans `PATCH settings` |
| Santé détaillée (bot, base, ffmpeg, yt-dlp, Piper, ingestions) | `GET /admin/api/health` |
| Ingestions récentes et échecs, toutes guildes | `GET /admin/api/ingestions` |
| Purger le cache, nettoyer les orphelins | `POST /admin/api/media/cache/purge`, `…/orphans/clean` |
| Journal consultable | persister l'audit `lc_server::admin_audit` (aujourd'hui dans les logs seulement) + `GET /admin/api/audit` |
| Paramètres du serveur modifiables à chaud | étendre `admin_runtime.json` au-delà du cookie TikTok ; les secrets, chemins et ports restent dans l'environnement (en lecture seule dans la maquette) |
| Droits par surface réglables | table `role_rights (surface, action, role, allowed)` lue par `GuildRole::can`, `lc_discord::rights::requirement` et `/ingest` ; `GET` / `PUT /admin/api/rights` ; notifier les sessions ouvertes |
| Vérifier le rôle à chaque envoi d'une extension | dans `POST /ingest`, relire le rôle de l'auteur du client (exclu ou invité : 403) |
| Réglages d'un appareil à distance | `PATCH /admin/api/overlay-clients/{id}/guild-settings` `{volume, mediaMuted, paused}` + message WebSocket ; aujourd'hui ces réglages ne se changent que depuis l'appareil |
| Mises à jour en direct | un flux WebSocket ou SSE d'administration plutôt que le rafraîchissement toutes les 2 s (budget de 100 requêtes / 15 min) |

## Quelle pile pour la vraie implémentation ?

Contraintes à garder : un seul binaire à déployer, CSP stricte
(`script-src 'self'`), pas de service Node à faire tourner en production, une
API JSON déjà testée (52 tests).

| Option | Pour | Contre |
|---|---|---|
| **A. Front typé léger, embarqué dans le binaire** — TypeScript + Preact (ou Lit) + Vite, sortie statique incluse avec `rust-embed`, types générés depuis les structures Rust (`ts-rs` ou schéma OpenAPI via `utoipa`) | un seul binaire, plus de `ADMIN_STATIC_DIR` à résoudre ; contrat API vérifié à la compilation des deux côtés ; composants réutilisables pour ~10 écrans ; ~15 Ko de runtime ; la maquette se transpose presque telle quelle | une étape Node au build (CI et image Docker), pas à l'exécution ; il faut typer les réponses aujourd'hui construites en `json!` |
| B. Rendu serveur — gabarits `askama` ou `maud` + htmx | pas de build JS, tout en Rust, très peu de code client | les routes JSON actuelles et leurs tests deviennent secondaires ; les mises à jour en direct (présence, antenne) et les dialogues riches sont plus laborieux ; htmx demande d'assouplir la CSP ou d'écrire beaucoup d'attributs |
| C. SPA complète — React ou SvelteKit, déployée à part | écosystème, bibliothèques de composants (shadcn/ui) | deuxième artefact à déployer et à versionner, CORS et cookies inter-origines, surdimensionné pour un outil à un ou deux utilisateurs |
| D. Garder du JS natif sans build (comme cette maquette) | zéro outillage | ne tient pas à cette taille : `app.js` fait déjà ~1 400 lignes sans appels réseau, sans types ni tests |

**Recommandation : A.** C'est le seul choix qui garde le déploiement actuel
(une image, un binaire) tout en rendant sûr un panneau de cette taille. Ordre
conseillé : d'abord l'authentification par session (elle conditionne l'exposition
du panneau hors loopback), puis les routes « à transposer », puis le flux temps
réel.

Indépendamment de l'option : remplacer le jeton en `localStorage` par un cookie
de session `HttpOnly; SameSite=Strict` (la défense CSRF par `Origin` existe
déjà), et retirer `style-src 'unsafe-inline'` de la CSP : la maquette n'a plus
aucun attribut `style` (jauges en `<progress>` natifs, mise en page par classes).

## Limites de la maquette

- Vérifiée dans Chromium (navigation, dialogues, recherche, largeur tablette
  768 px, aucune erreur console) ; pas testée dans Firefox ni Safari.
- Parcours clavier : focus visible, dialogues natifs `<dialog>`, lien
  d'évitement ; pas d'audit avec un lecteur d'écran.
- Chiffres, noms, versions de binaires et domaine sont inventés.
