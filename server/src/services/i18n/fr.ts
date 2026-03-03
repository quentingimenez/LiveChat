import { enLang } from './en';

export const frLang: typeof enLang = {
  commandError: 'Problème avec cette commande ! Veuillez vérifier les logs !',
  i18nLoaded: 'Langue française chargée !',
  serverStarted: 'Le serveur est lancé !',
  success: 'Succès !',
  error: 'Erreur !',
  notAllowed: 'Action non autorisée !',

  discordCommands: 'Chargement des commandes Discord',
  discordCommandLoaded: 'Commande chargée : /{{command}} ✅',
  discordInvite: 'Pour inviter le bot : {{link}}',
  discordBotReady: 'En ligne ! Connecté en tant que {{username}}',

  howToUseTitle: "Comment m'utiliser ?",
  howToUseDescription:
    'Utilisez `/pair-code` pour générer un code d’appairage à usage unique pour l’Overlay EXE ou la LiveChat Extension, puis saisissez-le dans l’application/extension. Ensuite utilisez `/msg`, `/cmsg`, `/dire`, `/cdire` pour envoyer des médias.',

  aliveCommand: 'dispo',
  aliveCommandDescription: 'Vérifiez si le bot est vivant',
  aliveCommandsAnswer: '{{username}}, Je suis en vie !',

  overlayCodeCommand: 'pair-code',
  overlayCodeCommandDescription: 'Générer un code d’appairage à usage unique pour Overlay EXE ou LiveChat Extension',
  overlayCodeCommandAnswerTitle: 'Code d’appairage Overlay',
  overlayCodeCommandAnswerDescription:
    'API_URL :\n```{{apiUrl}}```\nCode d’appairage :\n```{{code}}```\nExpire dans {{expiresIn}} minute(s).',

  memeAddCommand: 'meme-add',
  memeAddCommandDescription: 'Ajouter un media a la meme board persistante',
  memeAddCommandOptionURL: 'lien',
  memeAddCommandOptionURLDescription: 'Lien du media a enregistrer dans la meme board',
  memeAddCommandOptionMedia: 'media',
  memeAddCommandOptionMediaDescription: 'Media en piece jointe a enregistrer dans la meme board',
  memeAddCommandOptionTitle: 'titre',
  memeAddCommandOptionTitleDescription: 'Titre optionnel pour la meme board',
  memeAddCommandOptionForceRefresh: 'refresh',
  memeAddCommandOptionForceRefreshDescription: 'Ignorer le cache media et re-telecharger/re-transcoder la source',
  memeAddCommandMissingMedia: 'Veuillez fournir au moins une source media (lien ou piece jointe).',
  memeAddCommandAnswerCreated: 'Meme ajoute a la board.',
  memeAddCommandAnswerExists: 'Ce media est deja present dans la meme board.',

  sendCommand: 'msg',
  sendCommandDescription: 'Envoyer du contenu sur le stream',
  sendCommandOptionURL: 'lien',
  sendCommandOptionURLDescription: 'Lien du contenu sur le stream',
  sendCommandOptionText: 'texte',
  sendCommandOptionTextDescription: 'Texte à afficher',
  sendCommandOptionMedia: 'média',
  sendCommandOptionMediaDescription: 'Média à afficher',
  sendCommandOptionForceRefresh: 'refresh',
  sendCommandOptionForceRefreshDescription: 'Ignorer le cache média et re-télécharger/re-transcoder la source',
  sendCommandAnswer: 'Contenu reçu ! Il sera bientôt joué !',
  sendCommandMissingContent: 'Veuillez fournir au moins un texte, un média ou un lien.',
  sendCommandMediaError: 'Le média n’a pas pu être téléchargé ou normalisé.',
  sendCommandMediaErrorUnsupportedSource: 'Ce lien/source n’est pas supporté par le bot.',
  sendCommandMediaErrorPrivate:
    'Ce média est privé, restreint ou nécessite une authentification (cookies/compte) pour être récupéré.',
  sendCommandMediaErrorDrm: 'Ce média semble protégé par DRM et ne peut pas être téléchargé par le bot.',
  sendCommandMediaErrorNotFound: 'Média introuvable (lien supprimé, expiré ou invalide).',
  sendCommandMediaErrorTimeout: 'Le téléchargement a expiré (timeout). Réessayez dans quelques instants.',
  sendCommandMediaErrorTooLarge: 'Le média est trop volumineux (limite actuelle: {{maxSizeMb}} MB).',
  sendCommandMediaErrorCacheStorageLimit:
    'Le cache média non persistant est plein (limite: {{maxCacheTotalMb}} MB). Réessayez dans un instant.',
  sendCommandMediaErrorBoardStorageLimit:
    'Le stockage persistant de la mème board est plein (limite: {{maxBoardTotalMb}} MB). Supprimez des éléments avant un nouvel ajout.',
  sendCommandMediaErrorInvalidMedia: 'Le fichier téléchargé est invalide ou corrompu.',
  sendCommandMediaErrorTranscode: 'La conversion du média a échoué (codec/format).',
  sendCommandMediaErrorDownload: 'Le média n’a pas pu être téléchargé depuis cette source.',

  hideSendCommand: 'cmsg',
  hideSendCommandDescription: 'Envoyer du contenu sur le stream (mais caché 😈)',
  hideSendCommandOptionURL: 'lien',
  hideSendCommandOptionURLDescription: 'Lien du contenu sur le stream',
  hideSendCommandOptionText: 'texte',
  hideSendCommandOptionTextDescription: 'Texte à afficher',
  hideSendCommandOptionMedia: 'média',
  hideSendCommandOptionMediaDescription: 'Média à afficher',
  hideSendCommandOptionForceRefresh: 'refresh',
  hideSendCommandOptionForceRefreshDescription: 'Ignorer le cache média et re-télécharger/re-transcoder la source',
  hideSendCommandAnswer: 'Contenu reçu ! Il sera bientôt joué !',

  talkCommand: 'dire',
  talkCommandDescription: 'Demandez à un bot de dire quelque chose',
  talkCommandOptionText: 'texte',
  talkCommandOptionTextDescription: 'Texte à afficher',
  talkCommandOptionVoice: 'dire',
  talkCommandOptionVoiceDescription: 'Texte à dire',
  talkCommandAnswer: 'Contenu reçu ! Il sera bientôt joué !',
  talkCommandVoiceError: 'La génération vocale a échoué. Réessayez avec un texte plus court ou plus tard.',

  hideTalkCommand: 'cdire',
  hideTalkCommandDescription: 'Demandez à un bot de dire quelque chose (mais caché 😈)',
  hideTalkCommandOptionText: 'texte',
  hideTalkCommandOptionTextDescription: 'Texte à afficher',
  hideTalkCommandOptionVoice: 'dire',
  hideTalkCommandOptionVoiceDescription: 'Texte à dire',
  hideTalkCommandAnswer: 'Contenu reçu ! Il sera bientôt joué !',

  setDefaultTimeCommand: 'config-defaut',
  setDefaultTimeCommandDescription:
    "Définir le temps par défaut pour l'affichage (Par défaut : 5 secondes) (En secondes)",
  setDefaultTimeCommandOptionText: 'nombre',
  setDefaultTimeCommandOptionTextDescription: 'Nombre de secondes',
  setDefaultTimeCommandAnswer: 'Le temps par défaut est défini !',

  setMaxTimeCommand: 'config-max',
  setMaxTimeCommandDescription:
    "Définir le temps maximal pour l'affichage (En secondes) | 0 remet la valeur par défaut",
  setMaxTimeCommandOptionText: 'nombre',
  setMaxTimeCommandOptionTextDescription: 'Nombre de secondes',
  setMaxTimeCommandAnswer: 'Temps maximum défini !',

  setDisplayMediaFullCommand: 'config-displayfull',
  setDisplayMediaFullCommandDescription: 'Option legacy (conservée pour compatibilité)',
  setDisplayMediaFullCommandOptionText: 'value',
  setDisplayMediaFullCommandOptionTextDescription: 'Oui / Non',
  setDisplayMediaFullCommandAnswer: 'Valeur définie !',

  overlaysCommand: 'overlays',
  overlaysCommandDescription: 'Lister les overlays actuellement connectés',
  overlaysCommandAnswerTitle: 'Overlays connectés ({{count}})',
  overlaysCommandAnswerEmpty: "Aucun overlay n'est actuellement connecté sur ce serveur.",

  stopCommand: 'stop',
  stopCommandDescription: 'Interrompre la lecture en cours',
  stopCommandAnswer: 'Lecture interrompue !',
};
