# Re-audit ciblé P0/P1 — vérification 2 (2026-11-10)

Vérification indépendante des deux vagues de correctifs issues de l'audit 3.0.0 :

- vague 1 (P0) : commit `ab97b1c` — `p0-correctifs.md` ;
- vague 2 (P1) : commit `63e12f5` — `p1-correctifs.md`.

**Méthode** : deux vérificateurs indépendants (lecture seule, aucun code modifié),
n'ayant pas écrit les correctifs vérifiés : `audit-dynamique` pour le serveur
(protocole/WS, REST, admin, média, DB) et `overlay-bridge` pour l'overlay Tauri.
Chaque constat exigeait une preuve `fichier:ligne` ; le Lead a ensuite corrigé les
findings résiduels et rejoué les gates. Rapports détaillés :
`verification-2-server.md`, `verification-2-overlay.md`.

## 1. Verdicts par correctif

| Correctif | Verdict vérificateur | Après corrections |
|---|---|---|
| P0 (SSRF/LFI, ledger migrateur, session_mode, gate CI) | **TENU** sur pièces | TENU |
| COR-05 jetons hors URLs/logs | TENU | TENU |
| COR-06 WS Origin/plafonds/TimeoutLayer | TENU | TENU |
| COR-07 texte-seul, revoked, purge | TENU | TENU |
| COR-08 invariants DB, retry, migration 0002, `.old`, `mode=ro` | TENU | TENU |
| COR-09 confinement média, pipeline partagé, purge, TTL | **PARTIEL** | durci (voir §2) |
| COR-10 admin Bearer-only, rate-limit, compose | TENU | TENU |
| COR-11 overlay TLS/tweet/capabilities/updater/bundle | **7/8 TENUS**, updater PARTIEL | TENU (voir §3) |

## 2. Serveur — findings résiduels et corrections

Trois Majeurs, un Mineur, quatre Nits (0 Bloquant).

- **AUD-V2-S-001 (Majeur) — symlink hors racine** : le worker de purge suivait un
  lien dans `MEDIA_STORAGE_DIR`, canonicalisait la cible hors racine puis la
  supprimait.
  **Corrigé** : suppression uniquement via `confinement::confined_removal_path`
  (`symlink_metadata` no-follow, `canonicalize` + `starts_with` racine canonique,
  skip + warn hors racine) ; `list_variants` ignore les symlinks/non-fichiers ;
  tests de régression (symlink hors racine non supprimé).
- **AUD-V2-S-002 (Majeur) — chemins relatifs vs cwd / suppressions non confinées** :
  `storage_path` relatif était comparé au cwd via le repli de `normalized_path` ;
  la purge admin supprimait des chemins bruts hors racine. Risque de suppression
  de médias référencés après migration (conteneur/WORKDIR différent).
  **Corrigé** : `resolve_reference` résout les relatifs legacy contre la racine
  média (jamais le cwd), racine canonicalisée au démarrage, toutes les
  suppressions (worker + purge admin) passent par le garde confiné ; tests de
  régression (relatif `./data/media` avec cwd différent, fichier hors racine
  préservé — harnais `admin/tests.rs` ajusté en conséquence, exception Lead).
- **AUD-V2-S-004 (Mineur) — fichier partagé entre deux assets** : risque de course
  `remove_variants`.
  **Corrigé** : décompte des références par chemin canonique (suppression à la
  dernière référence) + verrou `lock_source_hash` partagé avec l'ingestion.
- **AUD-V2-S-003 (Majeur) — board hors `MediaService`** : pipeline partagé mais ni
  cache/quota/`record_failure`, ni suppression du fichier fraîchement téléchargé
  en cas de `forceRefresh` échoué. Effort M (unification), **non corrigé** :
  documenté en dette P2 `AUD-COR-13` (`docs/rust/lc-media.md`, backlog).
- Nits non bloquants reportés en P2/P3 : observabilité du worker détaché,
  rate-limit appliqué au streaming `Range`, exemptions de timeout par préfixe
  (vs routes exactes), branches `unique` mortes.

## 3. Overlay — findings résiduels et corrections

Deux Faibles, six Info (0 Élevée, 0 Moyenne, aucun bypass critique).

- **AUD-V2-O-001 (Faible) — updater joignable via le tray** : `announce_update`
  contournait `UPDATER_ENABLED_FOR_RELEASE=false` et déclenchait une requête réseau.
  **Corrigé** : entrée de menu grisée « Mises à jour désactivées pour cette
  version », `spawn_update_check` retourne immédiatement sans requête, tests.
- **AUD-V2-O-002 (Faible) — `clientToken` exposé aux fenêtres overlay/pairing** :
  **Corrigé** : `config_view_for_window` retire `clientToken` sauf pour la fenêtre
  board (qui en a besoin pour `authorizedFetch`), test de matrice.
- **O-003/O-004 (Info) — docs obsolètes** : **Corrigé** (`overlay-bridge.md`
  §12.2/§12.7, `dev/README.md`).
- Info restants : CSP large `connect/img/media`, config non atomique, diagnostics
  bridge en release, `withGlobalTauri` (aucun bypass constaté) → P2/P3.
- Binaire release `target/release/lc-overlay.exe` (18 486 272 o) : scan ASCII +
  UTF-16 sans `bridge-test.html`, `platform.twitter.com`, `twitter-widgets-js`.

## 4. Gates après corrections (HEAD `63e12f5` + surface V2)

| Gate | Résultat |
|---|---|
| `cargo fmt --all -- --check` | **0** |
| `cargo clippy --workspace --all-targets -- -D warnings` | **0** |
| `cargo test --workspace` | **0 échec** — `lc-server` lib **258 passed / 1 ignored**, `lc-media` 121 unit + 7 offline + 5 ssrf + doctest, `lc-db` 56 + doctest, `lc-playback` 19, `xtask` 50 / 1 ignored, `lc-overlay` 62 |
| `cargo deny check` | **0** (`advisories ok, bans ok, licenses ok, sources ok`) |

Logs : `.phase0/v2-gates.log`, `.phase0/v2-test.log`,
`.phase0/audit/task-46/gates.txt`.

## 5. Conclusion

Les correctifs P0 et P1 sont **défendables** : aucun Bloquant/Élevé résiduel après
re-audit, les Majeurs du vérificateur sont corrigés et couverts par des tests de
régression. Le confinement de la purge est désormais sûr pour les scénarios de
migration (chemins relatifs legacy, conteneurs, symlinks).

Restent avant release publique : dette P2 (`AUD-COR-13` unification board, nits
d'observabilité), P3, installation/upgrade NSIS en VM, clé minisign réelle, et
rejeu des goldens de parité legacy. Décision release 3.0.0 à prendre après ces
points ; `GO` local/LAN interne et migration contrôlée, `NO-GO` release publique
en l'état.
