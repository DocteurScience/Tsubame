# Design de Tsubame

Ce document résume la proposition de design faite avec Claude (maquettes sur un canvas, novembre 2026). Les sources des maquettes sont dans `docs/mockups/source/`. Ce sont des pages HTML autonomes : on peut les lire pour retrouver les dimensions, les espacements et le contenu de chaque écran. Les noms, comptes et textes de posts y sont fictifs.

## Écrans

| Maquette | Contenu |
|---|---|
| `Main.dc.html` | Accueil en trois volets : barre latérale, fil, volet de discussion (1280 px) |
| `Cartes.dc.html` | Variante retenue pour le fil : une carte par post, volet de lecture à droite avec sélecteur à trois positions (à droite, en dessous, masqué) |
| `Colonnes.dc.html` | Grand écran (2560 px) : plusieurs colonnes de cartes et volet de discussion |
| `Paliers.dc.html` | Schéma des quatre paliers d'adaptation |
| `Compose.dc.html` | Rédaction d'un post : images avec texte alternatif, vidéo, GIF, langue, étiquettes, réglages de réponses et de citations, brouillons, mention en cours de saisie |
| `Profil.dc.html` | Profil : bannière, statistiques, suivis communs, onglets (posts, réponses, médias, vidéos, j'aime, flux, listes, starter packs) |
| `Messages.dc.html` | Messagerie : conversations, demandes, conversation ouverte |
| `Explorer.dc.html` | Recherche, sujets du moment, personnes à suivre, starter packs |
| `Reglages.dc.html` | Page « Modération » des préférences : listes, comptes masqués ou bloqués, mots masqués, étiquettes de contenu, services d'étiquetage |
| `Mobile.dc.html` | Palier compact : une colonne et barre de navigation en bas |
| `Couverture.dc.html` | Correspondance entre les fonctions de Bluesky et les widgets GNOME |

La variante en cartes (`Cartes`, `Colonnes`) est celle que l'utilisateur a préférée. `Main.dc.html` montre l'autre option, un fil en liste simple.

## Paliers d'adaptation

Une seule colonne étirée serait mauvaise sur un grand écran. Le nombre de colonnes suit donc la largeur de la fenêtre, et chaque colonne reste entre 380 et 480 px.

| Palier | Largeur | Disposition |
|---|---|---|
| Compact | moins de 480 sp | une colonne, navigation en barre du bas (`AdwViewSwitcherBar`), discussion en page empilée |
| Moyen | environ 1280 px | barre latérale, une colonne, volet de discussion |
| Large | environ 1920 px (2K, ou 4K à 200 %) | barre latérale, 2 à 3 colonnes, volet |
| Très large | 2560 px et plus (4K à 100 %) | barre latérale, 4 colonnes ou plus, volet |

`AdwBreakpoint` gère les changements de disposition de la barre latérale et de la barre du bas. Le nombre de colonnes est du code à écrire : on divise la largeur disponible par la largeur d'une colonne.

## Cartes

- Une carte par post : fond de carte, bords arrondis (12 px dans la maquette), légère ombre, avatar rond de 40 px à gauche.
- Contenu : nom, heure, texte, médias avec pastille « ALT » si un texte alternatif existe, puis réponses, reposts et j'aime.
- La carte sélectionnée est marquée par un contour de la couleur d'accent, pas par un trait sur le côté.
- Le volet de lecture affiche le post ouvert en grand, puis ses réponses, aussi en cartes.

## Couverture fonctionnelle

Liste établie à partir des écrans déclarés dans `src/Navigation.tsx` du dépôt `bluesky-social/social-app`. Le détail de `src/screens` n'a pas pu être lu : des fonctions peuvent manquer.

**Lire**
- Fils épinglés (Following, Discover, fils personnalisés) : barre latérale.
- Nouveaux posts et actualisation : pilule flottante et bouton, F5.
- Fil de discussion et réponses : volet utilitaire, page empilée en mobile.
- Hashtags, sujets, flux vidéo : pages dans `AdwNavigationView`.
- Qui a aimé, reposté ou cité : `AdwDialog` avec liste de comptes.
- Signets : page de la barre latérale.

**Publier**
- Post, fil de posts, brouillons : `AdwDialog` (feuille basse en mobile), Ctrl+N.
- Images avec texte alternatif, vidéo, GIF, aperçu de lien : barre d'outils du dialogue, portail Fichiers.
- Langue, étiquettes de contenu, qui peut répondre ou citer : boutons à popover.
- Répondre, citer, reposter, aimer, partager, traduire : barre d'actions et menu ⋮.

**Découvrir**
- Recherche (posts, personnes, flux) : champ de recherche en en-tête, Ctrl+K.
- Tendances et comptes suggérés : listes encadrées.
- Starter packs (voir, créer, modifier) : cartes et assistant en pages.
- Flux (parcourir, épingler, réordonner) : page dédiée avec rangées déplaçables.

**Personnes et listes**
- Profil, onglets, abonnés, abonnements, suivis communs : page profil, `AdwToggleGroup`.
- Suivre, notifications d'activité, masquer, bloquer, signaler : boutons d'en-tête et menu ⋮.
- Modifier son profil (avatar, bannière, bio) : `AdwDialog`.
- Listes (créer, modifier, suivre) : page de la barre latérale.

**Messages et notifications**
- Conversations, demandes, groupes, réactions, partage de post : vue à trois volets.
- Notifications (toutes, mentions, réglages) : page de liste avec badge dans la barre latérale.
- Alertes hors de l'application : notifications de bureau via le portail.

**Modération et préférences**
- Comptes masqués et bloqués, listes de modération, mots masqués : `AdwPreferencesDialog`.
- Étiquettes de contenu, services d'étiquetage, boîte de modération : `AdwComboRow`, `AdwSwitchRow`.
- Compte (e-mail, mot de passe d'application, export, multi-comptes) : préférences, jeton gardé dans libsecret.
- Apparence, langues, accessibilité, intégrations externes : préférences, thème et accent du système.
- Aide et mentions légales : `AdwAboutDialog`.

## Limites de la proposition

- Les maquettes sont statiques et n'ont pas été testées sur de vrais appareils.
- Les écrans de notifications, de listes, de starter packs et de la plupart des préférences ne sont pas dessinés en détail : ils suivent les mêmes motifs que les écrans dessinés.
- Les couleurs des maquettes sont un point de départ. Dans l'application, elles viennent de libadwaita.
