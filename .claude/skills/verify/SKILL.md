---
name: verify
description: Use when the user asks to verify, double-check or trust an agent-written change to the LiveChat Rust code ("verify", "vérifie", "verification battery", "is this change trustworthy", "peux-tu vérifier ce changement"). Runs the local verification battery, mutation testing with kill-or-justify, then requires the adversarial review and the canary gate before any deploy.
---

# Vérifier un changement (pipeline de vérification)

Référence complète : [`docs/rust/verification.md`](../../../docs/rust/verification.md)
(table phase → technique → commande → gate, adaptations, nightly, pièges locaux).
Le protocole v2 est **gelé** ([`docs/rust/protocol-v2.md`](../../../docs/rust/protocol-v2.md),
38 fixtures intouchables).

## Workflow obligatoire pour tout changement écrit par un agent

1. **Spec d'abord** : énoncer le comportement attendu (fichier de spec, section de
   plan, ou énoncé écrit dans la PR) AVANT d'écrire le code.
2. **Tests défaillants d'abord** : le test qui prouve la règle doit exister et
   échouer avant l'implémentation (rouge → vert). Un test qui ne peut pas échouer
   n'est pas un test.
3. **Implémenter**, puis batterie locale :
   ```powershell
   cargo xtask verify --offline
   ```
   (fmt, clippy, nextest, doctests, cargo deny ; exit 0 exigé, `--locked --offline`
   partout.)
4. **Mutation — le test attrape-t-il quoi que ce soit ?**
   ```powershell
   cargo xtask verify --offline --mutants
   ```
   Le stage mutants termine en FAIL/exit 1 tant qu'il reste un mutant manqué :
   **kill-or-justify** — chaque mutant manqué tué par un test, ou justifié
   équivalent (exemple canonique : `playback_jobs.rs:211`, bras race-only, voir
   `docs/rust/verification.md` §5-6). La règle : 0 mutant produit manqué **non
   justifié** en `crates/` et `apps/` dans le stage local (diff de branche) ; les
   missed xtask sont tolérés mais comptés.
5. **Sabotage** : casser volontairement l'invariant **produit** (jamais le harnais),
   regarder le test nommé échouer avec un message exploitable, puis réverté
   byte-identique (vérifier le hash/MD5 du fichier). Consigner la preuve dans le
   corps du commit.
6. **Revue adversariale** : un second agent indépendant relit le diff avant tout
   commit — tests invincibles ou copies, sabotages qui cassent le harnais, réseau/
   DNS/sous-processus hors contrat, fichiers hors périmètre, générateurs qui évitent
   les entrées intéressantes. Chaque constat : `fichier:ligne` + test correctif.
7. **Canary avant tout déploiement** (jamais sans le go explicite de
   l'utilisateur) :
   ```powershell
   powershell -NoProfile -File scripts/release.ps1 deploy-server X.Y.Z -Canary
   ```
   Le skill [`/release`](../release/SKILL.md) décrit le déroulé complet
   et la fenêtre d'interruption résiduelle.

## Pièges locaux (Windows, PowerShell 5.1)

- cargo-mutants `--in-place` restaure les fichiers en LF → statuts fantômes ` M` à
  diff vide : normaliser en CRLF byte-level, ne jamais « git checkout ».
- proptest écrit `*.proptest-regressions*` à côté des tests : purger après un
  sabotage.
- PS 5.1 : pas de `&&` ; le compte `.Count` sur un objet unique ment — préférer
  `Group-Object`.
- Les nocturnes (`verification-nightly.yml`) ne sont jamais des merge gates : un
  nightly rouge se documente, se justifie ou se corrige, mais ne bloque pas une PR.
