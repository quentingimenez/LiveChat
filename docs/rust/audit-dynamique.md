# Audit dynamique du legacy — Phase 0 (rebuild Rust LiveChat)

> Mesures effectuées le **2026-11-09** sur `feat/rust-rebuild`, arbre de travail partagé, machine Windows x64.
> Objectif : établir des **faits mesurés** sur les 3 projets TS (server, overlay, extension) avant de porter le serveur en Rust.
> Toutes les commandes ont été exécutées avec `corepack pnpm` (server) ou `npm` (overlay/extension) ; aucun fichier source ni lockfile n'a été modifié.
> Artéfacts : `docs/rust/audit-server.json` (audit prod brut), `docs/rust/baseline.md` (protocole baseline détaillé), `docs/rust/prisma-dates.txt` (preuve littérale du format de dates).

## 1. Environnement mesuré

| Élément | Valeur |
|---|---|
| OS / arch | Windows, `win32 x64` |
| Node.js | **v24.12.0** |
| npm | 10.9.0 |
| corepack | 0.34.5 |
| pnpm (server) | **10.29.2** (champ `packageManager`) — seul accessible via `corepack pnpm` (pas de `pnpm` sur le PATH) |
| Prisma / @prisma/client | **5.22.0** / 5.22.0 (query engine `605197351a3c8bdd595af2d2a9bc3025bca48ea2`) |
| TypeScript | server **5.3.3** ; overlay/extension **5.9.3** |
| ESLint | server **8.56.0** ; overlay 9.39.3 ; extension 10.0.0 |
| Vitest | server 4.1.0 ; extension 4.0.18 (overlay = `node --test`) |
| Electron / electron-builder / electron-updater | **39.2.7** / 26.8.1 / 6.8.3 |
| Vite (extension) | 7.3.1 |
| Bash Git | `C:\Program Files\Git\bin\bash.exe` (utilisé pour les scripts shell overlay) |
| Docker | disponible (non utilisé pour cet audit) |

**Organisation** : monorepo « à la main », **sans `pnpm-workspace.yaml`**. `package.json` racine = scripts d'orchestration (`install:all`, `server:dev`, `overlay:verify`, `extension:check`, …) ; chaque sous-projet a son propre gestionnaire de paquets. `server/` = pnpm + `pnpm-lock.yaml` ; `overlay/` et `extension/` = npm + `package-lock.json`.

Dépendances directes résolues (extraits) :

- **server** : fastify **5.8.2**, socket.io **4.8.3**, discord.js **14.25.1**, @prisma/client **5.22.0**, zod **3.25.76**, date-fns 3.6.0, lodash **4.17.23**, dotenv 16.6.1, @t3-oss/env-core 0.8.0, pino-pretty 10.3.1, rosetty 3.1.32, `@livechat/overlay-protocol` **file:packages/overlay-protocol** (paquet local privé, protocole `1.2.0`) ; dev : tsx 4.21.0, tsc 5.3.3, vitest 4.1.0.
- **overlay** : electron 39.2.7, electron-builder 26.8.1, electron-updater 6.8.3, socket.io-client 4.8.3.
- **extension** : vite 7.3.1, vitest 4.0.18, typescript 5.9.3, eslint 10.0.0.

## 2. Installations (frozen / ci)

| Projet | Commande | Résultat | Durée | Paquets |
|---|---|---|---|---|
| server | `corepack pnpm install --frozen-lockfile` | **OK (0)** | 73,8 s (dont pnpm 58,3 s) | 447 |
| overlay | `npm ci` | **OK (0)** | 95,6 s | 435 |
| extension | `npm ci` | **OK (0)** | 69,2 s | 128 |

- `server` : `prisma generate` exécuté par le postinstall → client 5.22.0 + `query_engine-windows.dll.node` (19 261 952 o). pnpm signale **« Ignored build scripts: bufferutil@4.1.0, esbuild@0.27.3, unrs-resolver@1.11.1, utf-8-validate@6.0.6, zlib-sync@0.1.10 »** (`pnpm approve-builds` requis pour les exécuter) ; impact bénin ici (les modules natifs optionnels ne sont pas reconstruits).
- **Piège d'outillage** : `corepack pnpm install` dans `overlay/` échoue immédiatement avec `corepack : Unsupported package manager specification (traversal@1.0.0)` → le champ `packageManager` d'`overlay/package.json` est invalide. Contournement : `npm ci` (cohérent avec les scripts racine `overlay:*` qui utilisent `npm --prefix`). `extension/` n'a pas de `packageManager` → `npm ci` aussi.
- `npm ci` affiche un décompte de vulnérabilités légèrement différent de `npm audit` (39 vs 43 pour overlay). **Référence retenue : `npm audit`.**

## 3. Résultats des scripts réels

### server/ (`vitest`, `tsc`, `eslint`)

| Script | Commande exécutée | Exit | Durée | Résultat |
|---|---|---|---|---|
| `test` | `corepack pnpm exec vitest run` | **0** | 3,1 s | 4 fichiers / **50 tests passés** (overlayAuth 10, stringUtils 24, discord-utils 10, discordUserSync 6 ; duration 1,96 s) |
| `build` | `tsc -p tsconfig.json` | **2** | 3,1 s | **3 erreurs TS2339** (voir ci-dessous) |
| `lint` | `eslint src --ext .ts` (sans `--fix`) | **2** | 0,9 s | **crash de l'outil**, pas des violations |
| `start` | `node ./dist/src/index.js` | n/a | n/a | **inexploitable** (voir build/start) |

Erreurs de build exactes (`tsc 5.3.3`) :

```
src/loaders/socketLoader.ts(160,19): error TS2339: Property 'createdByDiscordUserId' does not exist on type 'SocketData'.
src/loaders/socketLoader.ts(169,90): error TS2339: Property 'message' does not exist on type '{}'.
src/loaders/socketLoader.ts(186,39): error TS2339: Property 'createdByDiscordUserId' does not exist on type 'SocketData'.
```

→ **le serveur legacy ne compile pas dans son état actuel.** `tsc` émet quand même `dist/` (pas de `noEmitOnError`). Constat supplémentaire : `tsc` produit `dist/index.js` (racine `src/`), alors que `package.json` `start` pointe sur `node ./dist/src/index.js` → **`start` est cassé même après correction des types** (mauvais chemin).

Crash lint exact (`eslint 8.56.0`, exécuté sans `--fix` pour ne pas modifier les sources) :

```
ESLint: 8.56.0
TypeError: Cannot set properties of undefined (setting 'defaultMeta')
    at ajvOrig (.../server/node_modules/@eslint/eslintrc/dist/eslintrc.cjs:1626:27)
NOT SUPPORTED: option missingRefs. Pass empty schema with $id that should be ignored to ajv.addSchema.
```

Cause probable : l'override pnpm `ajv ^8.17.1` impose ajv 8 à `@eslint/eslintrc@8.56` qui attend ajv 6 → **impossible de linter le serveur** dans l'état du lockfile. (Cohérent avec l'exception ajv documentée dans `server/SECURITY_EXCEPTIONS.md` ; en revanche l'affirmation « 0 vulnérabilité prod » de ce fichier est fausse, cf. §4.) ⚠ Le script officiel `pnpm lint` contient `--fix` : il **modifierait** les sources ; ne pas l'utiliser en audit.

### overlay/ (Electron, npm)

| Script | Exit | Durée | Résultat |
|---|---|---|---|
| `lint` | 0 | 4,6 s | OK (`eslint . --max-warnings=0`) |
| `typecheck` | 0 | 3,6 s | OK (3 × `tsc --noEmit`) |
| `build` | 0 | 4,3 s | OK (main + preload + renderer, 3 × tsc) |
| `check:renderer:esm` | 0 | 0,4 s | « Renderer ESM check passed » (pairing/overlay/board/preload-shared) |
| `test:unit` | 0 | 0,7 s | `node --test` : **62 tests passés, 0 échec** (duration_ms 303,74) |
| `check:lines` | **1** via `npm run` | — | **non exécutable sous Windows** : npm/cmd.exe n'exécute pas `./scripts/check-max-lines.sh` (`'.' n'est pas reconnu…`) |
| `check:lines` (contourné) | 0 | — | via `C:\Program Files\Git\bin\bash.exe` : « Line-count check passed (187 files) », max **250 lignes/fichier** |

### extension/ (Chromium MV3, npm)

| Script | Exit | Durée | Résultat |
|---|---|---|---|
| `typecheck` | 0 | 2,5 s | OK |
| `lint` | 0 | 7,9 s | OK |
| `test` | 0 | 1,6 s | **7 fichiers / 109 tests passés** (tiktok-capture 44, errors 19, url 17, messages 11, ingestClient 5, settings 3, +1) |
| `build` | 0 | 1,3 s | `vite build` → « ✓ built in 310ms », bundles `content` 23,88–28,97 kB |

**Bilan exécution** : extension et overlay sont verts (hors `check:lines` non portable sous Windows) ; **server = test vert, lint cassé, build cassé, start cassé**.

## 4. Audit des dépendances

### server — `corepack pnpm audit --prod --json` (sortie brute : `docs/rust/audit-server.json`)

- Exit **1** (vulnérabilités présentes). JSON valide : 61 advisories.
- **61 vulnérabilités prod : 32 high, 22 moderate, 7 low, 0 critical** (bloc `metadata` : 33 high / 23 moderate / 7 low).
- Par paquet : **undici 29**, **fast-uri 12**, **fastify 8**, engine.io 3, socket.io-parser 2, lodash 2, ws 2, zod 1, fast-copy 1, find-my-way 1.
- Exemples de correctifs exigés : `fastify >=5.12.2` (installé 5.8.2) — dont *auth bypass via malformed URLs* et *body schema validation bypass* ; `socket.io-parser >=4.2.7` ; `engine.io >=6.6.10` ; `undici >=7.30.1` ; `fast-uri >=3.1.9` ; `find-my-way >=9.7.0` ; `lodash >=4.17.24` ; `zod >=4.6.6` (major bump, non trivial) ; `ws >=8.21.0` ; `fast-copy >=3.1.0`.
- **Contradiction documentaire** : `server/SECURITY_EXCEPTIONS.md` (2026-02-20) affirme « `pnpm audit --prod` reports no known vulnerabilities » → **faux aujourd'hui** (61 advisories prod). Le port Rust supprime de facto cette dette (fastify/socket.io/undici ne seront plus embarqués) ; à documenter comme gain.

### overlay — `npm audit`

- **43 vulnérabilités** : **1 critical, 21 high, 21 moderate, 0 low**.
- **Prod uniquement** (`--omit=dev`) : **6** (5 high, 1 moderate, 0 critical) — directes : **electron-updater** (6.8.3, plage vulnérable 2.9.0–6.8.8) ; indirectes : builder-util-runtime, js-yaml, socket.io-parser, ws.
- Directes **production** high : **electron** (39.2.7 ; plage `<=42.11.15 || 43.0.0-alpha.1..43.0.0-beta.8`), **electron-builder** (26.8.1 ; plage `19.25.0 || 19.28.0..26.14.0`), **electron-updater**.
- Le **critical = tar** (indirect, chaîne electron-builder, dev/build) ; pas en prod.

### extension — `npm audit`

- **19 vulnérabilités** : **1 critical, 7 high, 10 moderate, 1 low**.
- **Prod uniquement** : **0**. Le critical est **vitest** (direct, dev ; arbitrary file read via @vitest/mocker), idem high vite/postcss/etc.

## 5. Baseline serveur (legacy TS)

Protocole complet, logs et artefacts : `docs/rust/baseline.md`. Résumé des mesures (Windows, machine de dev, sans Discord actif) :

| Mesure | Valeur |
|---|---|
| Migrations Prisma (base neuve, 12 migrations) | **2,18 s** |
| Migrations `migrate deploy` sans migration en attente | 2,1–3,84 s |
| Démarrage **dev** (`tsx src/index.ts`) → `GET /health` 200 | **14,42 s** (2ᵉ run) / **22,71 s** (1ᵉʳ run à froid) |
| Démarrage **prod-like** (`node dist/index.js`, build émis malgré erreurs TS) | **3,67 s** |
| RSS après 30 s (dev) | **145,2 Mo** (pic 216,4 Mo à 5 s) ; private 163,3 Mo |
| CPU cumulé après ~32 s (dev) | 9,27 s ; échantillon 5 s = **0 %** (idle) |
| Threads / handles / sockets TCP | 35 / 323 / 2 |
| Latence `GET /health` (50 req, keep-alive, après warmup ; dev) | **moyenne 1,06 ms, p50 0,91, p95 1,82, p99 1,87, min 0,63, max 3,10** |
| Latence `GET /health` (prod-like `dist`) | moyenne **0,76 ms**, p50 0,70, p95 1,15, p99 1,18 |
| Corps de réponse `/health` | `{"status":"ok"}` (HTTP 200) |

Combien de temps se passe-t-il après chargement des modules ? Les logs montrent `[DB] Connected to database` → `[BOOT] REST routes ready` en **~32 ms** (02.935 → 02.967). **L'essentiel du boot dev est le transpile/chargement tsx**, pas l'init applicative.

**Blocages/enseignements baseline** :
1. **Discord est obligatoire au boot** : sans `DISCORD_TOKEN` et `DISCORD_CLIENT_ID`, `createEnv` (@t3-oss/env-core + zod) jette `❌ Invalid environment variables: { DISCORD_TOKEN: [ 'Required' ], DISCORD_CLIENT_ID: [ 'Required' ] }` puis `Error: Invalid environment variables` (exit 1 en **7,05 s**). Le code `z.string()` accepte la chaîne vide, mais **PowerShell 7.3+ supprime une variable d'env affectée à `''`** ; pour mesurer, on a injecté des valeurs factices non vides (`baseline-disabled-token`, id numérique bidon). Résultat : le serveur démarre et répond 200, l'échec de login Discord survient **en asynchrone** (`void loadDiscord(...).catch(...)` non bloquant).
2. **Aucun fichier `.env` n'existe** dans `server/` ; les variables ont été injectées dans l'environnement du process (dotenv ne surcharge pas l'existant). Le `.env.example` indique `PORT=3333`, `API_URL=http://localhost:3333`, `DATABASE_URL=file:./sqlite.db`.
3. Le **build** doit être corrigé avant tout `start` (3 erreurs TS + mauvais chemin `dist/src/index.js`).
4. Aucun `dist/` livré dans le dépôt ; `dist/` régénéré pour la mesure puis supprimé.

## 6. CRITIQUE — format exact des dates Prisma/SQLite

Protocole : `server/.probe-dates.ts` (jetable, supprimé) exécuté via `tsx` après `prisma migrate deploy` sur une base jetable. Preuve littérale complète : **`docs/rust/prisma-dates.txt`**.

**Réponse : Prisma 5.22 + SQLite stocke les `DateTime` en `INTEGER` = millisecondes Unix epoch (UTC).**

Faits vérifiés :

1. **Déclaration** : colonnes `DATETIME` (`pragma_table_info('MediaAsset')` → `type = DATETIME`, `dflt_value = CURRENT_TIMESTAMP` pour `createdAt`/`lastAccessedAt`, `expiresAt` sans défaut).
2. **Stockage réel** : `typeof(createdAt)` = **`integer`** ; la valeur écrite pour `2024-01-02T03:04:05.678Z` est **`1704164645678`** (`CAST(createdAt AS TEXT)`), `hex()` = ASCII hex de cette chaîne de chiffres.
3. **Lecture via le client** : `createdAt` revient en objet JS `Date` ; `toISOString()` = `"2024-01-02T03:04:05.678Z"` — restitution **exacte à la milliseconde**, en UTC, indépendante du fuseau système.
4. **`$queryRawUnsafe('SELECT createdAt …')`** : renvoie des `Date` JS (Prisma applique la conversion des colonnes `DATETIME`), `JSON.stringify` → `"2024-01-02T03:04:05.678Z"`.
5. **Précision** : milliseconde seulement. `new Date(1704164645678.9012)` → `getTime() = 1704164645678` ; toute fraction sous-ms est tronquée avant stockage.
6. **Indépendance du fuseau** : même script avec `TZ=Europe/Paris` puis `TZ=UTC` → **valeurs explicites strictement identiques** (seules les valeurs `now()` diffèrent, car elles dépendent de l'instant réel). Aucune conversion locale n'intervient.
7. **Defaults** : `createdAt`/`lastAccessedAt` = `DEFAULT CURRENT_TIMESTAMP` au niveau SQL, mais Prisma **fournit `now()` côté client** → valeurs écrites en ms (ex. `…278Z`), pas en secondes.
8. **Valeurs TEXT possibles** (piège majeur) : un `INSERT` SQL brut omettant `createdAt` laisse jouer le défaut SQL → stockage **TEXT** `"2026-11-09 21:49:42"` (UTC, précision **seconde**, format espace, sans `T`/`Z`). Prisma **relit** ces valeurs comme `2026-11-09T21:49:42.000Z`. Un INSERT brut explicite `'2025-01-02 03:04:05'` est relu `"2025-01-02T03:04:05.000Z"` (interprété **UTC**).
9. **Tolérance du parseur Prisma** (SELECT brut) :
   - accepté : `2026-11-09 21:49:42` (UTC) ; `2026-11-09T21:49:42.123Z` ; `2026-11-09T21:49:42.123+02:00` (décalage respecté → `19:49:42.123Z`) ;
   - **rejeté** : `2026-11-09T21:49:42.123` (T sans Z/offset → `Inconsistent column data: Conversion failed: input contains invalid characters`) ; `2026-11-09 21:49:42.987` (espace + fraction → idem) ; réel `1704164645678.9011` (REAL → `Could not convert value … to type DateTime`).
   - Une seule valeur invalide dans une colonne `DATETIME` fait **échouer toute requête sélectionnant cette colonne** (`P2010 Raw query failed`).

**Conclusion** : le format canonique legacy est **entier 64 bits = epoch ms UTC**. Les formats TEXT (`YYYY-MM-DD HH:MM:SS` UTC, ISO 8601 avec Z/offset) ne peuvent apparaître que via des écritures SQL hors Prisma, mais le migrateur Rust doit savoir les lire pour rester compatible avec une base existante.

## 7. Implications concrètes pour le migrateur Rust

1. **Schéma SQLite** : garder les colonnes datetime en **`INTEGER` (i64, ms epoch UTC)**. Ne pas les convertir en `TEXT`/`TIMESTAMP` : sinon incompatibilité de lecture avec Prisma si les deux cohabitent, tri/index cassés, migration risquée. Rust : `i64` + `chrono::DateTime<Utc>::from_timestamp_millis`.
2. **Parseur tolérant** (compat bases existantes) : accepter (a) `i64` ms (canonique) ; (b) `TEXT "YYYY-MM-DD HH:MM:SS"` → interpréter **UTC** ; (c) RFC 3339/ISO 8601 avec `Z` ou offset (respecter l'offset) ; rejeter explicitement le reste avec log (équivalent du piège Prisma « input contains invalid characters »). Ne pas accepter `T` sans timezone.
3. **Écriture** : toujours générer `now()` **côté application en ms**, jamais via `DEFAULT CURRENT_TIMESTAMP` (précision seconde + type TEXT). Si des migrations SQL Rust sont écrites, ne pas déclarer de défaut `CURRENT_TIMESTAMP` sur les colonnes datetime.
4. **API HTTP/Socket** : les payloads JS exposent les dates via `Date.toISOString()` → RFC 3339 UTC avec **millisecondes** et suffixe `Z` (`2024-01-02T03:04:05.678Z`). Côté Rust : sérialiser avec `chrono::serde::ts_milliseconds` puis formater ISO (`to_rfc3339_opts(SecondsFormat::Millis, true)`) pour rester **byte-compatible** avec l'extension et l'overlay (protocole `overlay-protocol` v1.2.0).
5. **Ordre/tri/filtres** : les index (`MediaAsset_expiresAt_idx`, `status`) et les comparaisons de type `expiresAt < now()` fonctionnent sur des entiers → conserver `i64` et comparer avec `Utc::now().timestamp_millis()`. Attention aux bornes 2038 : les ms epoch sur i64 n'ont pas de problème Y2038 (contrairement à un stockage `INTEGER` 32 bits).
6. **Compat Prisma** : tant que la migration n'est pas terminée, les deux moteurs peuvent écrire des lignes différentes ; un lecteur Rust qui suppose uniquement `i64` **cassera** sur les lignes TEXT créées par des `INSERT` SQL bruts (défauts SQL, scripts d'exploitation, imports). Prévoir une passe de normalisation/validation.
7. **Dette supprimée par le port** : 61 advisories prod (fastify 8, undici 29, fast-uri 12, engine.io 3, socket.io-parser 2, lodash 2, ws 2, zod 1, fast-copy 1, find-my-way 1) et un serveur qui **ne compile pas** ; baseline à battre : démarrage prod-like 3,67 s (dev 14,4 s), RSS ~145–157 Mo, `/health` ~1 ms.
8. **Référence protocole** : `server/packages/overlay-protocol/src/index.ts` (v1.2.0) définit les événements socket (`overlay:play|stop|heartbeat|error|playback-state|meme-trigger|peers`) et les payloads (dont `durationSec`, `remainingMs`, `startOffsetSec`) ; voir aussi `docs/rust/protocol-v2.md` produit par l'équipe.

## 8. Limites de cet audit

- Mesures sur **une seule machine de dev Windows**, sans charge concurrente : les latences (~1 ms) sont des planchers, pas des SLA. Un bench Rust devra être comparé sur la même machine, même port, même scénario.
- La baseline « prod » utilise un `dist/` **émis malgré des erreurs de type** (tsc sans `noEmitOnError`) ; ce n'est pas un artefact de production valide, seulement une borne de démarrage.
- Discord désactivé par valeurs factices : ne mesure pas le coût du login/gateway Discord réel, ni les workers qui en dépendent.
- `pnpm audit --prod` dépend de la base advisories npm au 2026-11-09 ; les compteurs évolueront.
- Les scripts `overlay` `check:lines`/`verify` dépendent de bash (Git bash) — non portables en l'état sous Windows avec `npm run`.
