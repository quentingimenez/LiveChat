# Charte graphique LiveChat

Référence visuelle de LiveChat : logo, couleurs, typographie et icônes.
Elle s'applique à la board de l'overlay, aux icônes (tray, installeur,
fenêtres) et à l'extension navigateur. Le panneau d'administration, refait
plus tard, devra la suivre.

Hors périmètre : le rendu des médias à l'écran du stream (fenêtre overlay)
et les embeds Discord du bot ne changent pas.

## Le logo

Trois coins de cadrage figurent l'écran du stream. Le quatrième, en haut à
droite, est remplacé par une bulle ambre qui lance un média : un message qui
apparaît par-dessus ce que l'on regarde.

Il existe en deux dessins, sur une grille de 256 × 256 :

| Dessin | Utilisation | Coins | Bulle |
| --- | --- | --- | --- |
| Complet | 48 px et plus | trait de 18 | 116 × 96, rayon 28 |
| Simplifié | 32 px et moins | trait de 28 | 138 × 114, rayon 32 |

Le dessin simplifié grossit les traits et la bulle pour rester lisible à
16 px. Ne pas réduire le dessin complet sous 48 px.

### Couleurs du logo

- Bulle : Ambre `#F2B544`. Triangle de lecture : Encre ambre `#2A1D02`.
- Coins : couleur du texte du fond, Craie `#E9EBF3` sur fond sombre, Ardoise
  `#171B25` sur fond clair.
- La bulle reste ambre sur tous les fonds. Elle n'est jamais posée sur un fond
  ambre ou jaune.

### Icône d'application

Pour l'exécutable, l'installeur, les fenêtres et l'extension, le logo est
posé sur une tuile Ardoise `#171B25` aux coins arrondis (rayon 56 sur 256),
pour rester lisible sur une barre des tâches ou une barre d'outils claire
comme sombre. Marge intérieure : 36 unités pour le dessin complet, 22 pour le
dessin simplifié.

L'icône du tray n'a pas de tuile : elle suit le thème de la barre des tâches
de Windows (coins Craie sur barre sombre, Ardoise sur barre claire).

### Zone de protection et taille minimale

- Laisser autour du logo un espace libre d'au moins 1/8 de sa taille
  (32 unités sur la grille de 256).
- Taille minimale : 16 px, avec le dessin simplifié.

### À éviter

- Colorer les coins en ambre, ou la bulle dans une autre couleur.
- Tourner, déformer, ajouter une ombre, un contour ou un dégradé.
- Déplacer la bulle ou rétablir le quatrième coin.
- Utiliser le dessin complet sous 48 px.

## Couleurs

La palette est celle de la board (tokens CSS de
`apps/lc-overlay/web/app/styles/base.css`). Ardoise bleutée pour les
surfaces ; l'ambre est réservé à ce qui passe à l'écran : quand quelque chose
est ambre, c'est que ça passe à l'écran (lecture en cours, bouton Jouer,
bulle du logo).

Le tableau ci-dessous donne les valeurs du thème sombre, celui par défaut ; le
thème clair ne change que la valeur des tokens (voir « Thèmes »).| Nom | Token | Valeur | Rôle |
| --- | --- | --- | --- |
| Ardoise | `--slate-900` | `#171B25` | fond principal, tuile des icônes |
| Ardoise 800 | `--slate-800` | `#1E2330` | fond de la zone de contenu |
| Régie | `--slate-700` | `#262C3C` | surfaces, boutons, notifications |
| Régie claire | `--slate-600` | `#30384C` | survol, badges |
| Trait | `--line` | `#3A4259` | bordures, séparateurs |
| Craie | `--text` | `#E9EBF3` | texte, coins du logo sur fond sombre |
| Texte doux | `--text-soft` | `#A3ABC2` | texte secondaire |
| Texte discret | `--text-faint` | `#717A93` | indications, placeholders |
| Ambre | `--amber` | `#F2B544` | à l'écran : lecture, Jouer, bulle |
| Encre ambre | `--amber-ink` | `#2A1D02` | texte et pictos posés sur l'ambre |
| Rouge | `--red` | `#EC6A70` | erreurs, actions destructrices |
| Vert | `--green` | `#5FC9A3` | succès |
| Bleu | `--blue` | `#7FA8F5` | focus, information, liens |

Contrastes à respecter : texte courant en Craie ou Texte doux sur Ardoise ;
sur l'ambre, uniquement l'Encre ambre.

## Thèmes

L'interface existe en deux thèmes, sombre et clair. Le sombre reste le défaut ;
le clair se choisit dans Paramètres → Apparence (board) et dans la page
d'options de l'extension. Le troisième choix, « Système », suit le thème
clair/sombre de Windows (board) ou du navigateur (extension).

Les deux thèmes partagent la même charte : seuls les tokens changent de valeur,
les composants n'ont pas de règle propre. Côté board, le thème clair est porté
par `apps/lc-overlay/web/app/styles/theme-light.css` sur
`<html data-theme="light">` ; côté extension, par
`extension/src/styles/theme-light.css` sur `<html data-lce-theme="light">`.

| Nom | Token | Valeur claire | Rôle |
| --- | --- | --- | --- |
| Surface | `--slate-900` | `#EEF1F7` | fond principal |
| Surface claire | `--slate-800` | `#FFFFFF` | zone de contenu, cartes |
| Régie | `--slate-700` | `#F1F4FA` | boutons, notifications |
| Régie claire | `--slate-600` | `#E3E9F3` | survol, badges |
| Trait | `--line` | `#D5DCE8` | bordures, séparateurs |
| Ardoise | `--text` | `#171B25` | texte, coins du logo sur fond clair |
| Texte doux | `--text-soft` | `#4F5972` | texte secondaire |
| Texte discret | `--text-faint` | `#6F7A90` | indications, placeholders |

L'ambre ne change pas : il reste réservé à ce qui passe à l'écran, et sur
l'ambre on n'écrit qu'en Encre ambre. Sur surface claire, les accents qui
portaient du texte prennent leur variante foncée (`--amber-text`,
`--red-text`, `--green-text`, `--blue-text`) pour tenir le contraste ; les
aplats et les traits gardent la couleur vive de la charte.

Les coins du logo suivent la surface : `logo.svg` (Craie) sur la board sombre,
`logo-light.svg` (Ardoise) sur la board claire.

Deux surfaces restent sombres dans les deux thèmes, parce qu'elles montrent ce
qui part à l'écran : la fenêtre média de l'overlay et la carte d'identification
des écrans. Les widgets injectés dans les pages (boutons, toasts, modales)
suivent le thème de l'extension. Le bouton d'envoi, lui, porte le dessin
simplifié avec des coins en `currentColor` : il emprunte la couleur de texte de
la page hôte — Craie sur un YouTube sombre, Ardoise sur un YouTube clair — et
garde la bulle ambre sur la pastille sombre de TikTok comme sur un X clair.

## Typographie

Police système de Windows, sans police web (la board fonctionne hors ligne
sous une CSP stricte) :

```css
font-family: 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif;
```

- Texte : 14 px, interlignage 1,45.
- Titres de section : 17 px, graisse 600.
- Code d'appairage et valeurs techniques : `'Cascadia Mono', Consolas, monospace`.

Libellés en français, en casse de phrase (« Ouvrir la board », pas « Ouvrir La
Board »), sans majuscules décoratives. Un bouton dit ce qu'il fait ; le
message qui suit reprend le même verbe.

## Mise en page et rédaction

Les composants ne choisissent plus leurs marges ni leurs tailles au cas par
cas : `base.css` porte les tokens, les feuilles de fonctionnalité les
consomment.

- Espacement : `--space-1` à `--space-8` (4, 8, 12, 16, 20, 24, 32, 40 px).
  Les écrans composent avec ces pas, pas avec des valeurs inventées.
- Texte : `--text-xs` à `--text-xl` (12, 13, 14, 17, 20 px) ; interlignage
  `--leading-tight` (1,25) pour les titres, `--leading-base` (1,45) pour le
  corps.
- Longueur de ligne : `--measure` (62 caractères) pour les descriptions et les
  aides. Au-delà, l'œil perd le début de la ligne suivante.
- Retours à la ligne : `text-wrap: balance` sur les titres,
  `text-wrap: pretty` sur les paragraphes, espaces insécables avant `: ; ! ?`
  et entre un nombre et son unité. Pas de `text-align: justify` : à 62
  caractères, la justification crée des lézardes.
- Titres de cartes : deux lignes maximum (`-webkit-line-clamp: 2`), coupées à
  la fin d'un mot ; le titre complet reste dans l'infobulle et le tiroir de
  détail. La hauteur réservée est celle calculée par `masonry.js` : les deux
  doivent rester d'accord.
- Un seul bouton primaire (`is-primary`, bleu) par écran ou dialogue. L'ambre
  reste réservé à ce qui passe à l'antenne ; un formulaire n'a jamais d'ambre.
- Cibles d'au moins 32 px de haut, icônes seules toujours accompagnées d'un
  `aria-label` et d'une infobulle.
- Dialogues : en-tête et pied fixes, corps seul défilant, bouton de fermeture
  en haut à droite, action principale visible sans défiler.
- Rédaction : casse de phrase, registre « tu », un terme par concept
  (« média », « légende », « appareil », « écran »). Un bouton dit ce qu'il
  fait ; le message qui suit reprend le même verbe.

## Fichiers

Sources (dans `logo/`) :

| Fichier | Contenu |
| --- | --- |
| `livechat-logo-on-dark.svg` | dessin complet, coins Craie |
| `livechat-logo-on-light.svg` | dessin complet, coins Ardoise |
| `livechat-logo-small-on-dark.svg` | dessin simplifié, coins Craie |
| `livechat-logo-small-on-light.svg` | dessin simplifié, coins Ardoise |
| `livechat-logo-current-color.svg` | dessin complet, coins en `currentColor` (à intégrer dans une page) |
| `livechat-app-icon.svg` | tuile + dessin complet |
| `livechat-app-icon-small.svg` | tuile + dessin simplifié |

Le bouton d'envoi injecté par l'extension n'embarque aucun de ces fichiers : un
SVG posé dans une page hôte exigerait `web_accessible_resources` et une requête
réseau. `extension/src/content/brand-logo.ts` reprend le dessin simplifié en
balisage, avec les coins en `currentColor` (le bouton fait 16 px minimum).

Icônes générées :

| Fichier | Contenu | Utilisation |
| --- | --- | --- |
| `apps/lc-overlay/icons/icon.ico` | tuile, 16, 24, 32 (simplifié), 48, 64, 128, 256 | exécutable, installeur, fenêtres |
| `apps/lc-overlay/icons/icon.png` | tuile, 512 | icône du bundle Tauri |
| `apps/lc-overlay/icons/tray-dark.png` | simplifié, coins Craie, 32 | tray, barre des tâches sombre |
| `apps/lc-overlay/icons/tray-light.png` | simplifié, coins Ardoise, 32 | tray, barre des tâches claire |
| `extension/public/icons/icon-16.png` … `icon-32.png` | tuile, simplifié | barre d'outils du navigateur |
| `extension/public/icons/icon-48.png`, `icon-128.png` | tuile, complet | page des extensions, boutique |

Pour régénérer les icônes après une modification des SVG (Windows, avec
Microsoft Edge et Pillow), depuis PowerShell à la racine du dépôt :

```powershell
python docs/brand/render_icons.py
```
