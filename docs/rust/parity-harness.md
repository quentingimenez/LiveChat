# Harnais de parité legacy TypeScript ↔ cible Rust (Phase 9a)

Le harnais compare les réponses HTTP du serveur legacy (TypeScript/Fastify) à
celles de la cible Rust, route par route, **sans modifier ni l'un ni l'autre**.
Il est porté par la sous-commande `parity` du binaire `xtask` :

- `xtask parity capture` rejoue la suite de cas (v1 ou v2, selon `--suite`)
  contre un serveur et écrit un dossier **golden** (une réponse de référence par
  cas) ;
- `xtask parity compare` rejoue la même suite contre une autre cible et rapporte
  les différences, cas par cas, dans un rapport JSON et une table console.

Sources de vérité associées :

- **Inventaire REST** : [`docs/rust/migration-plan.md`](migration-plan.md)
  annexe A (26 routes v1 / 25 routes v2).
- **Surface HTTP cible** : [`docs/rust/server-core.md`](server-core.md).
- **Contrats admin** : [`docs/rust/admin-panel.md`](admin-panel.md) §6.
- **Suite de cas versionnée v1** : [`docs/rust/parity/cases/v1.json`](parity/cases/v1.json)
  (contrat legacy, référence métier).
- **Suite de cas versionnée v2** : [`docs/rust/parity/cases/v2.json`](parity/cases/v2.json)
  (cible Rust : évolutions voulues, voir §11).

## 1. Garanties

| Garantie | Mise en œuvre |
| --- | --- |
| Suites figées et embarquées | `cases/v1.json` et `cases/v2.json` sont incluses dans le binaire (`include_str!`), donc non modifiables par le serveur testé |
| Aucun effet de bord côté serveur | seules des requêtes de la suite sont envoyées ; aucune écriture, aucun nettoyage |
| Aucun secret persisté | en-têtes `authorization`/`cookie` jamais écrits ; jetons remplacés par `<redacted>` ; URLs signées normalisées |
| Goldens autoportants | chaque dossier golden contient `cases.json` + `manifest.json` (SHA-256 par fichier) |
| Comparaison normalisée | timestamps, UUID, chemins de stockage, URLs signées et valeurs volatiles sont neutralisés avant diff |
| Exécution reproductible | client HTTP dédié, délai par requête borné, redirections désactivées, sortie JSON stable |

## 2. Architecture

```text
docs/rust/parity/cases/v1.json           suite legacy embarquée (34 cas)
docs/rust/parity/cases/v2.json           suite cible v2 embarquée (13 cas)
        │
        │  xtask parity capture --base-url http://127.0.0.1:3000 [--suite v2]
        ▼
docs/rust/parity/golden-v1/ (ou golden-v2/)
  cases.json                             copie de la suite au moment de la capture
  manifest.json                          version, date, base_url, sha256 par golden
  <case_id>.json                         réponses de référence redactées
        │
        │  xtask parity compare --golden docs/rust/parity/golden-v1 \
        │                        --base-url http://127.0.0.1:8080
        ▼
rapport JSON (--report) + table console + code de sortie
```

La suite v1 est la référence métier (comportement legacy) ; la suite v2
documente les évolutions voulues de la cible Rust (§11). `--suite` sélectionne
la suite embarquée et le dossier golden par défaut (`golden-v1` / `golden-v2`).

| Module | Rôle |
| --- | --- |
| `xtask/src/parity/case.rs` | chargement/validation de la suite et règles de comparaison |
| `xtask/src/parity/normalize.rs` | normalisation des corps (placeholders, pointeurs ignorés) |
| `xtask/src/parity/request.rs` | préparation des requêtes, jetons, client HTTP, capture d'une réponse |
| `xtask/src/parity/capture.rs` | exécution de la suite, écriture des goldens et du manifeste |
| `xtask/src/parity/compare.rs` | rejeu, comparaison statut/en-têtes/corps, construction du rapport |
| `xtask/src/parity/report.rs` | rapport JSON + rendu console |
| `xtask/src/parity/cli.rs` | analyse manuelle des arguments (pas de `clap`) |

Le client HTTP utilise `reqwest` (rustls, sans suivi de redirection) déjà
présent dans les dépendances workspace ; aucune dépendance de test
supplémentaire.

## 3. Suites de cas (`cases/v1.json` et `cases/v2.json`)

La suite **v1** (34 cas) couvre le contrat legacy : racine et `/health`,
`/socket.io/` (426), route inconnue (404), `/overlay` (appairage,
`/media/:assetId`, meme-board), `/ingest` (appairage, ingestion) et `/admin`
(page, overview, réglages, jetons, codes d'appairage, stop/purge).

La suite **v2** (13 cas) cible le serveur Rust : en-têtes de sécurité,
`x-request-id`, `/health` v2, `426` strict sur `/socket.io/`, `404` normalisé,
`/overlay/config` absent, `/overlay/media` sans signature (`403`), auth
`Bearer` only et `429` de rate limiting. Elle sert de **golden de non-régression
de la cible** : capture depuis le serveur Rust, puis `compare` en CI (§7, §11).

Champs d'un cas :

| Champ | Sens |
| --- | --- |
| `id` | identifiant unique (`[a-z0-9_-]`) ; sert de nom de fichier golden |
| `description`, `note` | documentation lisible ; `note` signale une divergence connue |
| `method`, `path` | requête HTTP ; `{media_id}` est remplacé par `--media-id` |
| `body` | corps JSON optionnel |
| `headers` | en-têtes additionnels optionnels |
| `auth` | `none` (défaut), `admin`, `admin_invalid`, `overlay`, `ingest` |
| `require` | dépendances (`media_id`, `admin_token`, `overlay_token`, `ingest_token`, `rate_limit_probe`) ; un cas `rate_limit_probe` doit être le **dernier** de la suite |
| `status_alternatives` | statuts acceptés en plus de celui du golden |
| `compare` | règles de comparaison : `ignore`, `volatile`, `sort_arrays`, `ignore_body`, `allow_extra_keys`, `compare_headers` |

Comportement en cas de dépendance absente : le cas est **ignoré** (`skipped`)
et signalé dans le rapport, jamais compté comme échec. Un cas ignoré n'empêche
pas un code de sortie `0` tant que les cas exécutés sont conformes.

## 4. Normalisation

Avant comparaison, et avant écriture d'un golden, les valeurs suivantes sont
remplacées :

| Motif | Placeholder |
| --- | --- |
| ISO 8601 (`2025-01-01T00:00:00.000Z`) | `<timestamp>` |
| epoch millisecondes (plage 2001–2100) | `<timestamp_ms>` |
| UUID | `<uuid>` |
| URL/chemin de stockage (`./data/media/...`, `C:\...\data\media\...`) | `<storage_path>` |
| paramètres signés (`?sig=`, `&exp=`, `token=`, `mediaToken=`…) | `<redacted>` |
| `Bearer <jeton>` | `Bearer <redacted>` |
| champ d'objet nommé `token`, `secret`, `password`, `cookie`, `authorization` | `<redacted>` |

Les en-têtes capturés sont normalisés de la même façon, avec des placeholders
dédiés aux valeurs volatiles du rate limiting (les en-têtes sont normalisés à
la capture **et** à la comparaison) :

| En-tête | Traitement |
| --- | --- |
| `x-ratelimit-remaining` | `<rate_limit_remaining>` |
| `x-ratelimit-reset` | `<rate_limit_reset>` |
| `retry-after` | `<retry_after>` |
| `x-ratelimit-limit` | valeur stable (`100`), comparée telle quelle |
| `x-request-id` | UUID généré → `<uuid>` ; valeur fournie par le client conservée |
| `set-cookie` | jamais capturé (absent de l'allowlist d'en-têtes) |

Puis les règles du cas s'appliquent :

- `ignore: ["/chemin"]` : pointeur JSONPath ignoré (supprimé des deux côtés) ;
  `*` accepte un segment, `**` accepte zéro ou plusieurs segments ;
- `volatile: ["/chemin"]` : valeur remplacée par `"<volatile>"` des deux côtés ;
- `sort_arrays: ["/chemin"]` : tableaux triés avant comparaison ;
- `ignore_body: true` : seul le statut est comparé (corps non déterministe ou
  spécifique à un framework) ;
- `allow_extra_keys: true` : clés supplémentaires de la cible tolérées ;
- `compare_headers: ["content-type"]` : en-têtes comparés (vide = aucun).

## 5. Capturer les goldens depuis le legacy

Prérequis, sur la machine de développement legacy :

1. base SQLite Prisma à jour (au moins une guilde, un média `READY` rattaché à
   un job de lecture, des clients overlay/ingest) ;
2. `server/.env` renseigné (`ADMIN_UI_TOKEN`, `I18N`, éventuellement
   `ADMIN_UI_LOCAL_ONLY=false` si l'accès n'est pas loopback) ;
3. serveur legacy démarré (`pnpm dev` dans `server/`) ;
4. jetons overlay/ingest d'un client non révoqué, et un `--media-id` valide.

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$env:CARGO_BUILD_JOBS = "4"

cargo run -p xtask -- parity capture `
  --base-url http://127.0.0.1:3000 `
  --admin-token $env:ADMIN_UI_TOKEN `
  --overlay-token $env:OVERLAY_TOKEN `
  --ingest-token $env:INGEST_TOKEN `
  --media-id <assetId> `
  --out docs/rust/parity/golden-v1
```

Ordre de sortie : `docs/rust/parity/golden-v1/cases.json`, un
`<case_id>.json` par cas exécuté, puis `manifest.json` (SHA-256 de chaque
golden, date, URL de base, cas ignorés et raison).

**Ne jamais éditer un golden à la main.** Toute différence de comportement
acceptable doit être documentée dans `cases/v1.json` (`status_alternatives`,
`compare.ignore`, `compare.volatile`, `note`) ou dans ce runbook, puis les
goldens sont **recapturés**. Les goldens sont versionnés après revue du Lead.

**Suite v2** : il n'existe pas de legacy v2. La capture se fait depuis la cible
Rust avec `--suite v2` (dossier `docs/rust/parity/golden-v2/`) ; son rôle est la
non-régression du serveur Rust, pas la comparaison au legacy. Procédure
détaillée : [`parity/golden-v2/README.md`](parity/golden-v2/README.md).

## 6. Comparer une cible Rust

```powershell
# cible = serveur Rust démarré avec la base migrée
cargo run -p lc-server

cargo run -p xtask -- parity compare `
  --base-url http://127.0.0.1:8080 `
  --golden docs/rust/parity/golden-v1 `
  --report target/parity/rapport.json
```

Pour la suite v2, remplacer `--golden` par `docs/rust/parity/golden-v2` et
ajouter `--rate-limit-probe` : le cas `rate_limit_429` (dernier de la suite)
exige que la fenêtre de rate limit soit épuisée par la sonde avant le rejeu.

Codes de sortie :

| Code | Sens |
| --- | --- |
| `0` | tous les cas exécutés sont conformes |
| `1` | au moins une différence |
| `2` | erreur de configuration ou d'exécution (golden illisible, réseau, argument invalide) |

Exemple de rapport (extrait) :

```json
{
  "schema_version": 1,
  "harness_version": "3.0.0",
  "generated_at": "2026-11-10T01:02:03.456Z",
  "base_url": "http://127.0.0.1:8080",
  "golden_dir": "docs/rust/parity/golden-v1",
  "golden_label": "livechat-v1",
  "ignored_pointers": [],
  "summary": { "passed": 33, "failed": 1, "skipped": 0 },
  "cases": [
    {
      "case_id": "overlay_pair_consume_empty",
      "outcome": "pass",
      "expected_status": 400,
      "actual_status": 400,
      "diffs": [],
      "duration_ms": 3
    },
    {
      "case_id": "admin_overview_with_auth",
      "outcome": "fail",
      "expected_status": 200,
      "actual_status": 200,
      "diffs": [
        { "pointer": "/totals/guildCount", "golden": 2, "actual": 3 }
      ],
      "duration_ms": 5
    }
  ]
}
```

## 7. Utilisation en CI

```powershell
# 1. serveur cible lancé (base migrée + variables d'environnement)
# 2. comparaison ; échec bloquant si différences
cargo run -p xtask -- parity compare --base-url http://127.0.0.1:8080 --golden docs/rust/parity/golden-v1 --report target/parity/report.json
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
```

Le rapport JSON est l'artefact à archiver en CI ; la table console suffit pour
un diagnostic rapide. Une seconde exécution dédiée à la cible v2
(`--golden docs/rust/parity/golden-v2 --rate-limit-probe`) vérifie la
non-régression du serveur Rust indépendamment de la parité legacy.

## 8. Limites connues

- **Pas de WebSocket** : la suite ne couvre que le REST ; legacy monte un vrai
  Socket.IO, la cible répond 426 sur `/socket.io/` (capturé via
  `status_alternatives` et `ignore_body`).
- **État de base requis** : les cas `overlay_media_*` sont ignorés sans
  `--media-id` valide (média `READY` + job de lecture pour la guilde) ; les cas
  authentifiés sont ignorés sans jeton. Une base vide produit peu de cas
  comparables : capture sur une base représentative.
- **Jetons** : les jetons ne sont fournis que par variable d'environnement ou
  argument de commande, jamais stockés ; `admin_invalid` utilise une valeur
  sentinelle.
- **Divergences attendues** : CSP (`script-src` legacy accepte `'unsafe-inline'`,
  pas la cible), `/health` enrichi, corps Fastify par défaut sur 404, en-têtes
  de framework. Ils sont neutralisés par `compare.ignore`/`allow_extra_keys`
  dans la suite, pas par des règles globales.
- **URL média signée** : sans `exp`/`sig`/`clientId`, la cible répond
  `403 media_url_invalid` ; la suite v2 s'appuie sur ce comportement déterministe
  et ne requiert donc pas de jeton média.
- **Rate limiting** : la cible limite à 100 requêtes / 15 min par IP, avec
  `NODE_ENV=production` par défaut (pas d'exemption loopback) ; `/admin` est
  exempté et les réponses portent `x-ratelimit-*` + `retry-after`. Le cas
  `rate_limit_429` est **le dernier** de la suite v2 et exige
  `--rate-limit-probe` : la sonde épuise la fenêtre, ce qui rendrait
  non déterministes les cas suivants. La suite v1 évite les rafales.
- **Goldens pas encore capturés dans ce dépôt** : la première capture doit être
  faite sur la machine legacy, puis commitée après revue du Lead
  (`docs/rust/parity/golden-v1/README.md`).

## 9. Tests hors-ligne

Aucun test du harnais ne dépend du réseau ni d'un serveur externe :

- `MockServer` maison (`std::net::TcpListener` + thread, réponses servies dans
  l'ordre, `Connection: close`) ;
- tests du normaliseur (placeholders, pointeurs, `*`/`**`) ;
- tests du comparateur (statuts alternatifs, diffs imbriqués, clés
  supplémentaires, corps JSON/texte/vide) ;
- tests capture/compare de bout en bout (goldens + manifeste + SHA-256, roundtrip
  pass→diff, rapport écrit sur disque, table console) ;
- validation des suites embarquées (`livechat-v1` ≥30 cas, `livechat-v2` ≥10
  cas, dernier cas `rate_limit_probe`) ;
- extensions v2 : normalisation des en-têtes rate limit/`x-request-id`, sauts de
  cas (`requirement_gap`), sonde de rate limit (`429`) et parseur CLI
  (`--suite`, `--rate-limit-probe`).

```powershell
cargo test -p xtask        # 45 tests verts + 1 benchmark ignoré (migrateur inclus)
cargo fmt -p xtask -- --check
cargo clippy -p xtask --all-targets -- -D warnings
```

## 10. TODO Phase 9b

- première capture réelle sur la machine legacy et commit de
  `docs/rust/parity/golden-v1/` après revue ;
- première capture v2 depuis la cible Rust et commit de
  `docs/rust/parity/golden-v2/` après revue ;
- vérification des URLs média signées (HMAC) au-delà de la normalisation ;
- intégration CI du job `compare` (après démarrage du serveur Rust), y compris
  l'exécution v2 avec `--rate-limit-probe`.

## 11. Parité v2 : ce qui remplace quoi

Les écarts v1 → v2 ci-dessous sont des **évolutions voulues** de la cible, pas
des régressions. Chacune est couverte par un cas de `cases/v2.json`, rejoué en
CI (golden `golden-v2`). Les cas v1 restent la référence métier : une différence
non listée ici est une régression.

| Élément | Legacy v1 (référence) | Cible Rust v2 | Statut / cas v2 |
| --- | --- | --- | --- |
| CSP | `script-src 'self' 'unsafe-inline'` | `script-src 'self'` (pas d'inline), `frame-ancestors 'none'`, `object-src 'none'` | évolution voulue — `root_v2_response` |
| En-têtes de sécurité | partiels (helmet) | `x-content-type-options: nosniff`, `x-frame-options: DENY`, `referrer-policy: no-referrer`, `cross-origin-resource-policy: cross-origin`, `permissions-policy` — sur succès **et** erreurs | évolution voulue — `root_v2_response` |
| Traçabilité | absente | `x-request-id` généré (UUID) ou propagé depuis le client | évolution voulue — `request_id_generated`, `request_id_propagated` |
| `/health` | `{"status":"ok"}` | `{status,version,protocolVersion,db,discord,uptimeSec,activeJobs}`, toujours HTTP 200 | évolution voulue — `health_v2` (champs volatils neutralisés) |
| `/socket.io/` | vrai Socket.IO (200) | `426 {"error":"overlay_outdated","protocolVersion":2,"minOverlayVersion":"3.0.0"}` | évolution voulue — `socket_io_overlay_outdated_v2` (426 strict) |
| 404 / erreurs | corps Fastify par défaut | `{"error":"request_error"}` (4xx) / `{"error":"internal_error"}` (5xx) | évolution voulue — `unknown_route_normalized_404`, `method_not_allowed_normalized` |
| `/overlay/config` | présent (401 sans auth) | supprimé → 404 normalisé | évolution voulue — `overlay_config_absent` |
| URL média | chemin direct | URL signée HMAC ; sans signature `403 {"error":"media_url_invalid"}` | évolution voulue — `overlay_media_unsigned_forbidden` |
| Auth overlay | Bearer ou `?token=` | Bearer uniquement ; `?token=` ignoré (`401 unauthorized`) | évolution voulue — `overlay_meme_board_query_token_rejected` |
| Rate limiting | 100/15 min, exemptions de dev | 100/15 min, `/admin` exempté, `x-ratelimit-*` + `retry-after`, `429 {"error":"request_error"}` | évolution voulue — `rate_limit_429` |
