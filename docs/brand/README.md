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

| Nom | Token | Valeur | Rôle |
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

L'interface est sombre. Une surface claire n'apparaît que pour montrer le
logo sur fond clair (documentation, boutique de l'extension).

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
