# Goldens de parité v2 (`golden-v2/`)

Ce dossier est **généré** par `cargo run -p xtask -- parity capture --suite v2`
contre la **cible Rust** (`lc-server`). Il ne contient actuellement aucun golden
car la capture exige un serveur Rust démarré avec une base migrée représentative.

Contrairement à `golden-v1/` (référence legacy), ce dossier est un **golden de
non-régression** : il fige le comportement v2 voulu du serveur Rust. Les
évolutions v1 → v2 qu'il couvre sont documentées dans
[`docs/rust/parity-harness.md`](../../parity-harness.md) §11.

Documentation : [`docs/rust/parity-harness.md`](../../parity-harness.md).

## Contenu attendu

```text
golden-v2/
  cases.json          copie de la suite v2 au moment de la capture (autoportant)
  manifest.json       version du harnais, date, base_url, SHA-256 de chaque golden
  <case_id>.json      une réponse de référence par cas exécuté
  rate_limit_429.json référence du dernier cas (nécessite --rate-limit-probe)
```

Un golden contient le statut, les en-têtes filtrés (allowlist stable :
`content-security-policy`, `content-type`, `cross-origin-resource-policy`,
`permissions-policy`, `referrer-policy`, `x-content-type-options`,
`x-frame-options`, `x-request-id`, `x-ratelimit-*`, `retry-after`, …) et le
corps brut redacté. Les valeurs volatiles du rate limiting sont remplacées par
`<rate_limit_remaining>` / `<rate_limit_reset>` / `<retry_after>` à la capture.

## Procédure

1. démarrer la cible Rust avec une base migrée et une configuration
   représentative (`NODE_ENV` laissé à `production`, donc la limite de
   100 requêtes / 15 min s'applique aussi en loopback) ;
2. exécuter `parity capture --suite v2 --base-url <url> --rate-limit-probe`
   (les jetons admin/overlay/ingest sont optionnels et n'empêchent pas la
   capture : les cas concernés sont ignorés et listés dans le manifeste) ;
3. vérifier `manifest.json` (SHA-256, cas ignorés) et lire le diff attendu ;
4. faire relire le dossier par le Lead, puis committer `golden-v2/` ;
5. lancer `parity compare --golden docs/rust/parity/golden-v2 --rate-limit-probe`
   et archiver le rapport JSON.

## Rate limiting

Le cas `rate_limit_429` est **le dernier** de la suite v2 : `--rate-limit-probe`
envoie jusqu'à 120 requêtes `GET /health` pour épuiser la fenêtre de rate limit
avant de rejouer le cas. La sonde consomme la fenêtre courante ; pour recapturer
immédiatement, redémarrer le serveur (ou attendre 15 minutes). `/admin` est
exempté de la limite, donc les cas admin ne perturbent pas la capture.

## Règles

- ne jamais éditer un golden à la main ; toute évolution passe par une
  modification de `../cases/v2.json` puis une recapture ;
- ne jamais committer de jeton : `authorization`/`cookie` ne sont jamais
  écrits, `x-request-id` fourni par le client est conservé tel quel mais les
  UUID générés sont normalisés en `<uuid>` ;
- un changement de comportement v2 volontaire doit mettre à jour la suite
  (`status_alternatives`, `compare.ignore`, `note`), la documentation §11 **et**
  les goldens dans le même changement relu ;
- `golden-v2/` ne remplace pas `golden-v1/` : la parité legacy reste vérifiée
  séparément.
