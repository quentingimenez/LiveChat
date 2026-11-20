# Politique de confidentialité — extension LiveChat

Dernière mise à jour : 20 novembre 2026.

Cette politique décrit le traitement des données par l'extension de navigateur
« LiveChat Extension » (`fholhpodpecihbgpnfplnjolnkphgdpa`). L'extension n'a pas
de serveur propre : elle envoie les médias que vous choisissez à **votre**
instance LiveChat, dont vous êtes l'exploitant.

## Données traitées

| Donnée | Où elle va | Pourquoi |
| --- | --- | --- |
| Réglages (URL du serveur, jeton d'ingestion, identifiant de guilde, nom et avatar d'auteur) | `chrome.storage.local` de votre navigateur uniquement | Savoir où envoyer les médias et signer les envois |
| URL du post, texte et durée d'affichage d'un média | Le serveur que vous avez configuré, via `POST /ingest` | Ajouter le média au board et à l'overlay |
| URL directe du fichier média détectée sur TikTok | En mémoire dans l'onglet, jamais enregistrée | Récupérer le fichier envoyé par le site plutôt que de le retélécharger |

Aucun de ces éléments n'est transmis à l'éditeur de l'extension, à Google ou à
un autre tiers. L'extension n'utilise ni mesure d'audience, ni publicité, ni code
distant.

## Conservation

Les réglages restent dans votre navigateur jusqu'à leur modification, la
désinstallation de l'extension ou la suppression de ses données. Les médias
envoyés sont conservés par votre serveur LiveChat, selon sa configuration
(board, cache, historique).

## Vos droits

Vous pouvez consulter et supprimer les réglages à tout moment : désinstaller
l'extension, ou effacer ses données depuis `chrome://extensions`. Les données
envoyées à votre serveur se suppriment depuis ce serveur (board, historique) ou
en contactant son exploitant.

## Contact

Quentingimenez — <https://github.com/quentingimenez/LiveChat/issues>.

## Modifications

Toute évolution de cette politique sera publiée dans ce fichier, avec la date de
mise à jour ci-dessus.
