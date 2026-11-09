# Baseline serveur legacy (Phase 0) — protocole, mesures brutes, limites

> Mesures du **2026-11-09** (~22:46–22:52 heure locale), branche `feat/rust-rebuild`, Windows x64, Node v24.12.0.
> But : disposer d'une référence chiffrée du serveur TS actuel pour comparer le futur serveur Rust.
> Rapport de synthèse : `docs/rust/audit-dynamique.md` ; preuve du format de dates : `docs/rust/prisma-dates.txt`.

## 1. Objet mesuré

`server/` = Fastify 5 + Socket.IO 4.8 + Prisma 5.22/SQLite + Discord.js 14. Point d'entrée `src/index.ts` → `runServer()` (Fastify, Prisma, workers de purge, Socket.IO, Helmet, CORS, rate-limit 100 req/15 min, routes `/`, `/health`, `/admin`, `/overlay`, `/ingest`, scheduler de playback, chargement Discord **asynchrone non bloquant**).

- Port : **3399** (`env.PORT`), listen `::`.
- `/health` → HTTP 200, corps **`{"status":"ok"}`**.
- Audio/vidéo : stockage disque `MEDIA_STORAGE_DIR` (créé au boot) ; aucun média requis pour `/health`.

## 2. Environnement de test

Le dépôt **ne contient pas de `server/.env`** (volontairement) : les variables ont été injectées dans l'environnement du process, dotenv ne surchargeant pas les variables existantes.

```
PORT=3399
API_URL=http://localhost:3399
ADMIN_UI_TOKEN=test-token
DATABASE_URL=file:./tmp-baseline.db
MEDIA_STORAGE_DIR=./tmp-baseline-media
NODE_ENV=development
LOG=info
DISCORD_TOKEN=baseline-disabled-token
DISCORD_CLIENT_ID=000000000000000000
```

**Pourquoi des valeurs Discord factices ?** `src/services/env.ts` (`@t3-oss/env-core` + zod) déclare `DISCORD_TOKEN` et `DISCORD_CLIENT_ID` en `z.string()` **requis** : sans elles, le process meurt au boot. Mesuré :

```
❌ Invalid environment variables: { DISCORD_TOKEN: [ 'Required' ], DISCORD_CLIENT_ID: [ 'Required' ] }
Error: Invalid environment variables
    at createEnv (.../@t3-oss/env-core/dist/index.js:29:15)
    at Object.<anonymous> (.../src/services/env.ts:152:31)
```

→ **exit 1 en 7,05 s**. `z.string()` accepte aussi `''`, mais **PowerShell 7.3+ supprime une variable d'environnement affectée à `''`** ; on utilise donc des valeurs non vides. Avec elles : `/health` répond 200 ; le login Discord échoue **en asynchrone** car `void loadDiscord(fastify).catch(...)` n'est pas attendu par `setReady()`. Le coût réel du gateway Discord n'est donc **pas** mesuré ici.

## 3. Protocole

1. `server/` : `corepack pnpm exec prisma migrate deploy` (base `file:./tmp-baseline.db`, 12 migrations).
2. Lancement dev réel (équivalent du script `dev` = `pnpm migration:up && tsx ./src/index.ts`, mais `pnpm` n'est pas sur le PATH : `node node_modules/tsx/dist/cli.mjs ./src/index.ts`), en arrière-plan, stdout/stderr redirigés.
3. Sondage `GET http://127.0.0.1:3399/health` toutes les 100 ms ; chrono arrêté au premier HTTP 200 → *temps jusqu'à /health*.
4. Attente 30 s, puis `Get-Process` sur le PID réel qui écoute (le PID « wrapper » de `Start-Process` n'est pas le serveur en dev/tsx sur cette machine) : RSS (`WorkingSet64`), mémoire privée, CPU cumulé, threads, handles, sockets TCP.
5. Échantillon CPU 5 s (delta CPU / 5 s).
6. Latence : 5 requêtes de warmup puis **50 `GET /health`** en keep-alive (`fetch` Node 24, undici), mesure `performance.now()` → moyenne, p50, p95, p99, min, max.
7. Arrêt du process (`taskkill /T /F`), vérification que le port n'écoute plus, suppression de `prisma/tmp-baseline.db*` et `./tmp-baseline-media`.
8. Répété en mode **prod-like** : `tsc -p tsconfig.json` (exit 2 mais émet `dist/`), `node dist/index.js`, mêmes mesures (boot 3 s après boot pour RSS, puis latence).

Artefacts temporaires (hors dépôt) : `%TEMP%\lc-audit\` — `prisma-migrate*.log`, `server-baseline.out.log`, `server-dist.out.log`, `baseline-result.json`, `measure-health.mjs`. Rien n'est laissé dans le dépôt (ni base, ni média, ni `dist/`, ni script de probe).

## 4. Résultats bruts

### 4.1 Migrations Prisma

| Cas | Exit | Durée |
|---|---|---|
| Base neuve, 12 migrations appliquées | 0 | **2,18 s** |
| Base à jour (`No pending migrations`) | 0 | 2,1 / 3,33 / 3,84 s |

Log : `12 migrations found in prisma/migrations` … `All migrations have been successfully applied.`

### 4.2 Démarrage

| Mode | Boot → `GET /health` 200 | Remarque |
|---|---|---|
| dev (`tsx src/index.ts`), 1ᵉʳ lancement (cache froid) | **22,71 s** | inclut transpile/chargement tsx |
| dev (`tsx src/index.ts`), relance | **14,42 s** | cache chaud |
| prod-like (`node dist/index.js`) | **3,67 s** | `dist/` émis malgré 3 erreurs TS (voir §6) |

Timeline des logs dev (extrait, PID 33900) — noter que l'init applicative est très rapide une fois les modules chargés :

```
[22:48:02.935] INFO (33900): [DB] Connected to database
[22:48:02.945] INFO (33900): [BOOT] Prisma connected
[22:48:02.946] INFO (33900): [BOOT] Media storage ready
[22:48:02.946] INFO (33900): [BOOT] Media cache worker started
[22:48:02.946] INFO (33900): [BOOT] Playback job purge worker started
[22:48:02.946] INFO (33900): [BOOT] Pairing code purge worker started
[22:48:02.946] INFO (33900): [BOOT] Discord user sync worker started
[22:48:02.950] INFO (33900): [BOOT] Socket.IO server initialized
[22:48:02.955] INFO (33900): [BOOT] Security headers (Helmet) registered
[22:48:02.956] INFO (33900): [BOOT] Rate limiting registered (100 req/15min, admin routes exempt)
[22:48:02.958] INFO (33900): [BOOT] CORS registered
[22:48:02.959] INFO (33900): [BOOT] I18N loaded
[22:48:02.965] INFO (33900): [BOOT] Playback scheduler initialized
[22:48:02.966] INFO (33900): [BOOT] Socket loader ready
[22:48:02.966] INFO (33900): [REST] Admin Routes loaded (/admin)
[22:48:02.967] INFO (33900): [REST] Overlay Routes loaded (/overlay)
[22:48:02.967] INFO (33900): [REST] Ingest Routes loaded (/ingest)
[22:48:02.967] INFO (33900): [BOOT] REST routes ready
```

→ `[DB] connected` → `REST routes ready` = **32 ms**. Le temps de boot est dominé par le chargement/transpilation des modules (surtout en dev tsx). Le serveur logue aussi `[SECURITY] Development mode - allowing all CORS origins` (attendu avec `NODE_ENV=development`).

### 4.3 Ressources après 30 s (dev)

| Métrique | Valeur |
|---|---|
| RSS (WorkingSet) | **145,2 Mo** (pic mesuré à 5 s : 216,4 Mo — montée puis relâche GC) |
| Mémoire privée | 163,3 Mo |
| CPU cumulé (depuis le démarrage, ~32 s) | 9,27 s |
| CPU sur échantillon 5 s | **0 %** (idle) |
| Threads | 35 |
| Handles | 323 |
| Sockets TCP | 2 (port 3399 en écoute + une connexion cliente du sondage) |
| RSS prod-like 3 s après boot | 157,1 Mo |

### 4.4 Latence `GET /health`

Mode dev, 50 requêtes keep-alive après 5 warmup :

| moyenne | p50 | p95 | p99 | min | max |
|---|---|---|---|---|---|
| **1,06 ms** | 0,91 ms | **1,82 ms** | 1,87 ms | 0,63 ms | 3,10 ms |

Mode prod-like (`node dist/index.js`) :

| moyenne | p50 | p95 | p99 | min | max |
|---|---|---|---|---|---|
| **0,76 ms** | 0,70 ms | **1,15 ms** | 1,18 ms | 0,49 ms | 1,62 ms |

Toutes les réponses : HTTP 200, corps `{"status":"ok"}`.

## 5. Enseignements / pièges rencontrés

- **PID** : en dev, `Start-Process node cli.mjs` crée un process dont l'écouteur effectif a un **autre PID** ; mesurer `Get-NetTCPConnection -LocalPort 3399` → `OwningProcess` (sinon RSS/CPU faux ou process introuvable).
- **PowerShell stderr + `$ErrorActionPreference='Stop'`** : `corepack … 2>&1 | Out-File` déclenche `NativeCommandError` et termine tout le script dès qu'une commande native écrit sur stderr (même si elle réussit). → ne pas mettre `Stop` pendant les scripts natifs.
- **Variables d'env vides** : `$env:VAR=''` supprime la variable sous PowerShell 7.3+ (donc zod « Required » malgré une affectation apparente).
- **`[System.Net.Http.HttpClient]` indisponible** dans ce shell (type .NET non chargé) → la mesure de latence in-process a été refaite avec un script Node (`fetch`), plus fiable.
- **`dist/`** : `tsc` émet malgré les erreurs, mais le script `start` pointe sur `dist/src/index.js` alors que l'émission est `dist/index.js` → `start` cassé.

## 6. Limites

- Machine de dev unique, aucun trafic concurrent : les latences sont des **planchers**.
- Le mode prod-like n'est **pas** un artefact de production valide (build en erreur, voir `docs/rust/audit-dynamique.md` §3).
- Discord désactivé (valeurs factices) : login/gateway/workers Discord non exercés.
- Un seul passage par scénario (pas de médiane inter-runs) ; la variance observée sur le boot dev (14,4 s vs 22,7 s) est un effet de cache disque/tsx.
- `/health` ne touche pas la base : ne mesure pas la latence Prisma/SQLite ; il faudrait une route DB-backed pour cela (non présente).
