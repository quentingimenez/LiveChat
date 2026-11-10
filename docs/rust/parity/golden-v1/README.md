# Goldens de parité v1 (`golden-v1/`)

Ce dossier est **généré** par `cargo run -p xtask -- parity capture` ; il ne
contient actuellement aucun golden car la capture exige un serveur legacy
TypeScript démarré avec une base représentative, ce qui n'est pas disponible
dans cet environnement de build.

Documentation : [`docs/rust/parity-harness.md`](../../parity-harness.md).

## Contenu attendu

```text
golden-v1/
  cases.json          copie de la suite au moment de la capture (dossier autoportant)
  manifest.json       version du harnais, date, base_url, SHA-256 de chaque golden
  <case_id>.json      une réponse de référence par cas exécuté
```

Un golden contient le statut, les en-têtes filtrés (allowlist stable :
`content-type`, `location`, `retry-after`, `x-ratelimit-*`, `content-range`,
`accept-ranges`) et le corps brut redacté de la réponse.

## Procédure

1. sur la machine legacy : base Prisma à jour (guilde, média `READY` rattaché à
   un job, clients overlay/ingest), `server/.env` renseigné, `pnpm dev` lancé ;
2. exécuter `parity capture` avec `--base-url`, les trois jetons et un
   `--media-id` valide (voir le runbook) ;
3. vérifier `manifest.json` (SHA-256, cas ignorés) et lire le diff attendu ;
4. faire relire le dossier par le Lead, puis committer `golden-v1/` ;
5. lancer `parity compare` contre la cible Rust et archiver le rapport JSON.

## Règles

- ne jamais éditer un golden à la main ; toute évolution passe par une
  modification de `../cases/v1.json` puis une recapture ;
- ne jamais committer de jeton : `authorization`/`cookie` ne sont jamais
  écrits, les URLs signées sont normalisées en `<redacted>` ;
- un changement de comportement legacy documenté doit mettre à jour la suite
  (`status_alternatives`, `compare.ignore`, `note`) **et** les goldens dans le
  même changement relu.
