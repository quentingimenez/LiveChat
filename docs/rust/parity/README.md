# Parité legacy TypeScript ↔ cible Rust (Phase 9a)

Ce dossier contient les artefacts du harnais de parité du binaire `xtask`.
Documentation complète et limites : [`docs/rust/parity-harness.md`](../parity-harness.md).

```text
docs/rust/parity/
  cases/v1.json      suite legacy versionnée (embarquée dans le binaire xtask)
  cases/v2.json      suite cible v2 versionnée (embarquée dans le binaire xtask)
  golden-v1/         goldens capturés depuis le serveur legacy (README dédié)
  golden-v2/         goldens de non-régression v2 capturés depuis la cible Rust
```

La suite v1 est la référence métier (comportement legacy). La suite v2 fige les
évolutions voulues du serveur Rust (CSP/`x-request-id`/`429`/erreurs
normalisées, `/overlay/config` supprimé, `/overlay/media` signé, Bearer only) ;
le détail « ce qui remplace quoi » est en §11 de la documentation.

## Commandes

```powershell
# capturer les réponses de référence depuis le legacy TypeScript (suite v1)
cargo run -p xtask -- parity capture --base-url http://127.0.0.1:3000 `
  --admin-token $env:ADMIN_UI_TOKEN --overlay-token $env:OVERLAY_TOKEN `
  --ingest-token $env:INGEST_TOKEN --media-id <assetId> `
  --out docs/rust/parity/golden-v1

# comparer la cible Rust aux goldens v1
cargo run -p xtask -- parity compare --base-url http://127.0.0.1:8080 `
  --golden docs/rust/parity/golden-v1 --report target/parity/report.json

# capturer puis rejouer la suite v2 (non-régression de la cible Rust)
cargo run -p xtask -- parity capture --base-url http://127.0.0.1:8080 `
  --suite v2 --rate-limit-probe --out docs/rust/parity/golden-v2
cargo run -p xtask -- parity compare --base-url http://127.0.0.1:8080 `
  --golden docs/rust/parity/golden-v2 --rate-limit-probe `
  --report target/parity/report-v2.json
```

`--suite` sélectionne la suite embarquée et le dossier golden par défaut
(`golden-v1` / `golden-v2`). `--rate-limit-probe` épuise la fenêtre de rate
limit avant le dernier cas v2 (`rate_limit_429`).

Codes de sortie de `compare` : `0` conforme, `1` différences, `2` erreur
d'exécution. Les cas authentifiés ou dépendant d'un média sont ignorés (et
signalés) lorsque les jetons ou `--media-id` manquent.

## Politique

- `cases/v1.json` et `cases/v2.json` sont les sources de vérité des requêtes ;
  toute divergence attendue doit être documentée (`status_alternatives`,
  `compare.ignore`, `compare.volatile`, `note`) et non contournée par une règle
  globale ;
- les goldens ne s'éditent jamais à la main : recapturer avec `parity capture`
  après revue du Lead ;
- aucun jeton, cookie ou URL signée n'est écrit dans un golden ; `set-cookie`
  n'est jamais capturé et les valeurs volatiles de rate limit sont remplacées
  par des placeholders.
