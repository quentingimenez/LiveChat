## Problème

Le job `publish-webstore` a échoué lors de la publication de `extension-v0.2.2` :

```
upload: SUCCEEDED — version 0.2.2
X Wait for the upload to be processed
the upload was not processed within 5 minutes
```

L'étape d'attente bouclait sur `lastAsyncUploadState` renvoyé par `fetchStatus`.
Or l'upload v2 répond `SUCCEEDED` de façon synchrone et l'API omet ce champ
ensuite : la boucle ne pouvait jamais aboutir, et la soumission à la relecture
(`:publish`) n'était jamais appelée.

## Correctif

- L'étape d'upload expose son `uploadState` dans `$GITHUB_OUTPUT`.
- L'étape d'attente ne s'exécute plus que si l'API annonce un traitement
  asynchrone (`IN_PROGRESS`).
- Si le champ reste absent au bout de 5 minutes, la CI avertit puis tente la
  publication au lieu d'échouer : c'est `:publish` qui tranche.
- `extension/README.md` documente ce comportement.

## Tests

- `python -c "yaml.safe_load(...)"` sur `.github/workflows/extension.yml` : jobs
  `build`, `release`, `publish-webstore`, 7 étapes conformes.
- Le job corrigé sera exercé au prochain tag (`extension-v0.2.3`) ; la version
  0.2.2 a été soumise manuellement via l'API (`state: PENDING_REVIEW`).
