# Infrastructure Rust — toolchain du poste (Phase 0)

> **Statut** : opérationnel. Marqueurs : `.phase0/toolchain-ok` (créé par `infra-rust`), `.phase0/msvc-ok` (créé par le Lead).
> **Branche** : `feat/rust-rebuild`. **Date** : 2026-11-09.
> **Périmètre** : poste Windows de développement (`x86_64-pc-windows-msvc`), CI GitHub Actions.

## 1. Résumé

| Élément | Version / état |
|---|---|
| `rustc` | 1.99.1 (58cf8f9c3 2026-10-13) |
| `cargo` | 1.99.1 (5f94df478 2026-08-27) |
| `rustup` | 1.29.3 (edbc831ce 2026-11-02) |
| Toolchain | `stable-x86_64-pc-windows-msvc` (canal `stable`, profil `default`) |
| Composants | `rustfmt`, `clippy` (+ `rust-std`, `rust-docs`, `cargo`, `rustc`) |
| `cargo-nextest` | 0.9.151 (6b3f99457 2026-11-02) |
| `cargo-deny` | 0.20.2 |
| `cargo-audit` | 0.22.2 |
| MSVC / linker | Build Tools 2022 17.14.44, toolset 14.44.35207 — **OK** (voir `.phase0/msvc-ok`) |
| `cargo check --workspace` | **OK** (0,1 s à chaud) |
| `cargo build --workspace` | **OK** (lien MSVC réel) |
| `cargo test --workspace` | **OK** (0 test) |
| `cargo fmt --all -- --check` | **OK** |
| `cargo clippy --workspace --all-targets -- -D warnings` | **OK** (0 warning) |
| `cargo deny check` | **OK** (advisories, bans, licenses, sources) |
| `cargo audit` | **OK** (1430 advisories chargées, 9 dépendances scannées, 0 vulnérabilité) |
| `cargo nextest run --workspace` | **OK** avec `--no-tests=pass` (0 test, 9 binaires) |

## 2. Chemins installés

| Quoi | Chemin |
|---|---|
| Binaires cargo/rustc | `C:\Users\SuperKnacky\.cargo\bin` (`%USERPROFILE%\.cargo\bin`) |
| `CARGO_HOME` | `C:\Users\SuperKnacky\.cargo` |
| `RUSTUP_HOME` | `C:\Users\SuperKnacky\.rustup` |
| Log d'installation rustup | `.phase0/rustup-init.log` |
| Log d'installation des outils | `.phase0/install-tools.log` (96 Ko) |
| Marqueur toolchain | `.phase0/toolchain-ok` |
| Marqueur linker (Lead) | `.phase0/msvc-ok` |

`%USERPROFILE%\.cargo\bin` a été ajouté au **PATH utilisateur** (et préfixé dans chaque session de travail de l'agent).
L'ajout a été fait avec `[Environment]::SetEnvironmentVariable('Path', $new, 'User')` — **pas `setx`**, qui tronque à 1024 caractères. Longueur du PATH utilisateur : 1735 → 1766 caractères, aucune troncature.

## 3. Installation réalisée (reproductible)

### 3.1 rustup + toolchain

```powershell
# 1. Téléchargement
$base = 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc'
Invoke-WebRequest "$base/rustup-init.exe"        -OutFile .phase0\rustup-init.exe
Invoke-WebRequest "$base/rustup-init.exe.sha256" -OutFile .phase0\rustup-init.exe.sha256

# 2. Vérification d'intégrité (obligatoire)
(Get-FileHash .phase0\rustup-init.exe -Algorithm SHA256).Hash.ToLower()
# attendu : 38027d51d69a5d05493627a0634dbea52d6cd793c8f072d2373d3f4163eec7db

# 3. Installation non interactive
.\.phase0\rustup-init.exe -y --profile default `
    --default-toolchain stable-x86_64-pc-windows-msvc --no-modify-path

# 4. PATH utilisateur (sans setx)
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
[Environment]::SetEnvironmentVariable('Path', "$userPath;$cargoBin", 'User')

# 5. PATH de la session courante
$env:Path = "$cargoBin;$env:Path"
```

Durées mesurées : téléchargement ≈ 14 s (12 839 424 octets), **installation rustup + toolchain = 42,1 s** (sortie 0).
L'installeur a été supprimé de `.phase0/` après vérification pour ne laisser que logs et marqueurs ; le SHA256 ci-dessus permet de le retélécharger.

### 3.2 Composants de formatage/lint

```powershell
rustup component add rustfmt clippy   # déjà présents via le profil default, sortie 0
```

### 3.3 Outils cargo

```powershell
cargo install cargo-nextest cargo-deny cargo-audit --locked --jobs 8
```

Durée totale : **630,9 s (10 min 31 s)** ; sortie 0, « Successfully installed cargo-nextest, cargo-deny, cargo-audit ».
Détail : nextest 4 min 54 s, deny 7 min 38 s, audit 10 min 30 s (les trois compilés séquentiellement).
`--jobs 8` : le poste est partagé entre plusieurs agents, on évite la saturation des 16 threads.
Journal complet : `.phase0/install-tools.log`.

Vérification :

```powershell
cargo nextest --version   # cargo-nextest 0.9.151 (6b3f99457 2026-11-02)
cargo deny --version      # cargo-deny 0.20.2
cargo audit --version     # cargo-audit-audit 0.22.2
```

## 4. MSVC / linker

Le Lead a installé **Visual Studio Build Tools 2022** (UAC) : `17.14.44`, toolset `14.44.35207`,
`link.exe` = `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x64\link.exe`.
Marqueur : `.phase0/msvc-ok`.

Points vérifiés côté Rust :
- `rustc` cible `x86_64-pc-windows-msvc` et localise le linker MSVC via `vswhere`/registre — aucun `vcvars64.bat` à sourcer.
- **Piège** : `link.exe` présent dans le PATH est celui de **Git for Windows** (`C:\Program Files\Git\usr\bin\link.exe`), qui n'est pas un linker. Inoffensif : rustc n'utilise pas ce binaire. Ne pas le mettre en tête de PATH par erreur si un jour le PATH manuel est reconstruit.
- Preuve de bout en bout : `cargo build --workspace` et `cargo test --workspace` **exécutent** les binaires (donc édition de liens + exécution OK).

## 5. Squelette du workspace créé

```
Cargo.toml                  workspace: resolver 2, members crates/*, apps/*, xtask, exclude = ["spikes"]
Cargo.lock                  versionné ; en Phase 0 il ne contenait que les 9 paquets workspace,
                            puis il s'enrichit au fil des dépendances ajoutées par les crates
rust-toolchain.toml         canal stable, profil default, composants rustfmt + clippy
rustfmt.toml                edition 2021, max_width 100
clippy.toml                 msrv 1.85, seuils args/complexité
deny.toml                   licences autorisées, advisories deny, yanked deny, wildcards deny, sources deny
.cargo/config.toml          [build] jobs = 8, [net] retry = 3
.gitignore                  + /target, /spikes/*/target, /.phase0/
.github/workflows/rust.yml  CI Rust (fmt, clippy, nextest linux+windows, deny, audit)
```

Crates (toutes **minimales** en Phase 0 : `#![forbid(unsafe_code)]`, doc de crate, `[lints] workspace = true`, aucun contenu métier) :

| Chemin | Type | Rôle cible | Phase |
|---|---|---|---|
| `crates/lc-protocol` | lib | types/enveloppe protocole v2, fixtures golden | 1 |
| `crates/lc-domain` | lib | entités, ids, erreurs métier, temps | 1 |
| `crates/lc-db` | lib | sqlx/SQLite, repositories, migrations | 2 |
| `crates/lc-media` | lib | resolver, yt-dlp/ffmpeg, transcodage, cache | 5 |
| `crates/lc-discord` | lib | bot Discord, commandes, i18n | 6 |
| `apps/lc-server` | bin | binaire axum : REST + WS v2 + workers | 3/4 |
| `apps/lc-overlay` | lib `rlib` | shell Tauri 2 + bridge (POC Phase 0) | 8 |
| `xtask` | bin | migrateur, harnais de parité, release | 2/9 |

### 5.1 Politique de lints (Cargo.toml racine)

```toml
[workspace.lints.rust]
unsafe_code = "forbid"

[workspace.lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
panic = "deny"
```

Conséquences pratiques pour les agents des Phases 1+ :
- tout `unwrap()` / `expect()` / `panic!()` hors test est une **erreur de compilation** ;
- dans les tests, ajouter `#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` sur le module de test ;
- `apps/lc-overlay` devra **abaisser** `forbid(unsafe_code)` en `deny` dans son `lib.rs` le jour où le bloc Win32 isolé (topmost/click-through) sera écrit — décision actée dans le plan (§3) : `forbid` maintenant, dérogation ciblée en Phase 8.

### 5.2 `[workspace.dependencies]`

Déjà déclarées (versions précisées, résolution effective en Phase 1 quand des crates les utiliseront) :
`serde` (+derive), `serde_json`, `tokio` (rt-multi-thread, macros, net, time, sync, signal, fs), `tracing`,
`thiserror` 2, `anyhow`, `uuid` (v4+serde), `sha2`, `hmac`, `subtle`, `rand` 0.9, `base64` 0.22, `url` 2,
plus les 6 crates locales en `path`.

> **Résolution effective, au fil de la Phase 1** : en Phase 0 aucune crate ne consommait ces dépendances, donc
> `Cargo.lock` ne contenait que les 9 crates du workspace et ces versions n'étaient pas validées. Depuis, `lc-protocol`
> a ajouté `serde` + `serde_json` et le lock a été régénéré (`serde` 1.0.229, `serde_json` 1.0.154, `serde_core`,
> `serde_derive`, `syn` 3.0.10, `proc-macro2`, `quote`, `unicode-ident`, `itoa`, `memchr`, `zmij`). Les autres
> versions seront validées par la même mécanique au fur et à mesure.
>
> **Règle lock** : après tout ajout de dépendance, lancer `cargo check --workspace` **sans** `--locked` pour
> régénérer `Cargo.lock`, puis committer le lock. La CI utilise `--locked` et **échouera volontairement** si le lock
> est en retard — c'est le garde-fou prévu. Vérifié après régénération : `cargo check --workspace --locked` OK.

## 6. CI — `.github/workflows/rust.yml`

Déclencheurs : `push` sur `feat/rust-rebuild` et `main`, `pull_request`, `workflow_dispatch` — filtrés sur
`crates/**`, `apps/**`, `xtask/**`, les fichiers de config Rust et le workflow lui-même.

| Job | Runner | Contenu |
|---|---|---|
| `fmt` | ubuntu | `cargo fmt --all -- --check` |
| `clippy` | ubuntu | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` |
| `test` | ubuntu + windows (matrice) | `cargo nextest run --workspace --all-features --locked --no-fail-fast --no-tests=pass` puis `cargo test --workspace --doc --locked` |
| `deny` | ubuntu | `EmbarkStudios/cargo-deny-action@v2` (`cargo deny check`) |
| `audit` | ubuntu | `cargo audit`, **`continue-on-error: true`** (non bloquant en Phase 0/1) |

Points d'attention intégrés :
- `--no-tests=pass` sur nextest : sans lui, `cargo nextest run` sort en **code 4** (« no tests to run ») tant que la Phase 1 n'a pas ajouté de tests — vérifié localement.
- nextest n'exécute pas les doctests → étape `cargo test --doc` séparée.
- `--locked` partout : si `Cargo.lock` n'est pas à jour après l'ajout d'une dépendance, la CI casse volontairement (le lock doit être committé).
- `cargo audit` s'appuie sur `taiki-e/install-action@cargo-audit` (pas d'installation depuis les sources).
- `concurrency.cancel-in-progress` et `timeout-minutes` sur chaque job.

À ajouter plus tard (hors Phase 0) : `sqlx prepare --check` (Phase 2), `cargo build --release` + budget binaire (Phase 9), et suppression de `server.yml`/`overlay-ci.yml` après le cutover.

## 7. Vérifications exécutées sur le poste

| Commande | Résultat | Durée |
|---|---|---|
| `cargo check --workspace` | OK, 9 crates | 1,5 s à froid / 0,1 s à chaud |
| `cargo build --workspace` | OK (lien MSVC) | 0,5 s |
| `cargo test --workspace` | OK, 0 test | < 1 s |
| `cargo fmt --all -- --check` | OK (aucun diff) | — |
| `cargo clippy --workspace --all-targets -- -D warnings` | OK, 0 warning | 1,5 s |
| `cargo nextest run --workspace --no-tests=pass` | OK, 0 test / 9 binaires | 0,04 s |
| `cargo deny check` | OK (`advisories ok, bans ok, licenses ok, sources ok`) | — |
| `cargo audit` | OK, 0 vulnérabilité | premier run avec fetch de l'advisory-db |
| `cargo check --workspace` (re-vérifié après ajout `serde`/`serde_json` par `lc-protocol`) | OK, lock régénéré | 9,5 s (1re compilation) |
| `cargo check --workspace --locked` (état final, lock à jour) | OK | 0,05 s |

## 8. Blocages, limites et leçons

- **Aucun blocage bloquant** : toolchain, linker, outils et CI sont prêts pour la Phase 1.
- **Incident corrigé** : le test de linker du Lead (`cargo init .phase0/linktest`) a été **auto-ajouté** dans `members`
  du `Cargo.toml` racine par cargo, et compilé dans tous les `cargo check`/`clippy`. Corrigé : entrée retirée,
  `.phase0/linktest/` et `linktest.*` supprimés. `.phase0/` ne contient plus que logs et marqueurs.
  → Règle pour la suite : **ne jamais lancer `cargo init`/`cargo new` sous un workspace existant** ; utiliser
  `cargo init --name X` dans un dossier explicitement hors workspace, ou écrire le `Cargo.toml` à la main.
- **`rand = "0.9"`** a été retenu (et non 0.8) pour les besoins HMAC/tokens ; à confirmer lors de la première
  utilisation en Phase 1. Les versions CR de `crates.io` vues au moment de l'installation : serde 1.0.229,
  tokio 1.53.1, uuid 1.24.0, url 2.5.7, base64 0.22.1, sha2 0.10.9, subtle 2.6.1, thiserror 2.0.21, rand 0.10.3 (majeure suivante).
- **`rust-toolchain.toml` sur le canal `stable`** (pas de version épinglée) : rustup met à jour la toolchain au fil
  de l'eau. C'est le choix du plan (« stable »), mais cela signifie qu'une régression rustc/stable peut arriver ;
  en cas de problème, épingler `channel = "1.99.1"` dans `rust-toolchain.toml`.
- **MSRV déclaré `1.85`** dans `[workspace.package]` et `clippy.toml` : non testé (local et CI utilisent stable 1.99.1).
  À vérifier seulement si un build MSRV devient un objectif.
- **`apps/lc-overlay` est une `rlib` vide**, sans dépendance Tauri : volontaire pour que `cargo check --workspace`
  reste instantané. Le POC Tauri (Phase 0) vit hors workspace (worktree/spike) ; la structure Tauri définitive
  (`staticlib`/`cdylib` + bin) arrive en Phase 8.
- **Poste partagé** : `.cargo/config.toml` plafonne à 8 jobs. Si plusieurs agents compilent simultanément, préférer
  `cargo check -p <crate>` et laisser le Lead lancer les commandes globales (`nextest`, `clippy`).
- Les avertissements `license-not-encountered` de cargo-deny ont été neutralisés (`unused-allowed-license = "allow"`)
  parce que la liste blanche est pré-provisionnée pour les dépendances futures.

## 9. Comment relancer / vérifier

```powershell
# Toute session PowerShell neuve : le PATH utilisateur est déjà en place.
# Si l'on est dans un contexte qui ne le recharge pas :
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

# Vérifications rapides
rustc -V ; cargo -V ; cargo nextest --version ; cargo deny --version ; cargo audit --version
cargo check --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace --no-tests=pass
cargo deny check
cargo audit
```

Réinstaller la toolchain de zéro :

```powershell
rustup self uninstall -y
# puis refaire la §3.1 (téléchargement + SHA256 + rustup-init + PATH)
rustup component add rustfmt clippy
cargo install cargo-nextest cargo-deny cargo-audit --locked --jobs 8
```

## 10. Handoff Phase 1

Ce qui est prêt pour les agents suivants :
- marqueur `.phase0/toolchain-ok` présent → le fan-out peut démarrer ;
- workspace compilant, lints globaux actifs, CI prête ;
- les crates encore vides sont **libres** : leur propriétaire ajoute `[dependencies]`, le code, les tests
  (`lc-protocol` était déjà repris par `protocole-v2` au moment de la rédaction) ;
- contrat de propriété : les `Cargo.toml` de crates appartiennent aux agents propriétaires des crates ; le
  `Cargo.toml` **racine**, les fichiers de config Rust et le workflow `rust.yml` appartiennent à `infra-rust`/Lead.

Ce qui reste à faire (Phase 1+) :
1. `lc-protocol` : types v2 + fixtures golden (agent `protocole-v2`).
2. `lc-domain` : entités/erreurs/temps.
3. Ajouter `sqlx prepare --check` à `rust.yml` dès que `lc-db` utilise sqlx (Phase 2).
4. Ajouter un job `build --release` + contrôle de budget binaire quand le serveur compile pour de vrai (Phase 9).
5. Supprimer/remplacer les workflows JS historiques lors du cutover uniquement.
