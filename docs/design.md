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

Dans le code (`src/columns.rs`, `src/window.rs`) :

- Le widget `ColumnsView` place les colonnes ouvertes puis le volet de discussion. Il garde au volet au moins 440 px et affiche autant de colonnes de 380 à 480 px que le reste le permet. Le volet prend la place restante. Les colonnes qui ne tiennent pas sont masquées. La colonne choisie dans la barre latérale, ou avec Ctrl+Page suivante et Ctrl+Page précédente, reste toujours visible.
- Sous 1120 sp, la barre latérale, une colonne et le volet ne tiennent plus côte à côte : il ne reste qu'une colonne et la discussion s'ouvre en page empilée (`AdwNavigationView`). Ce seuil est un palier intermédiaire entre « compact » et « moyen ».
- Sous 480 sp (compact), la barre latérale disparaît au profit de la barre du bas (Accueil, Explorer, Notifications, Messages). Le titre de la colonne devient un menu pour changer de colonne, comme « Following ▾ » dans `Mobile.dc.html`.
- Le volet de lecture se place à droite des colonnes, en dessous, ou se masque, avec le sélecteur de la maquette `Cartes` dans son en-tête (et dans celui de la page empilée quand il est masqué). Le choix est enregistré dans GSettings (clé `pane-position`). En dessous ou masqué, les colonnes occupent toute la largeur : leur cadre peut dépasser 480 px, pas les cartes. Le sélecteur utilise des boutons liés ; il passera à `AdwToggleGroup` quand la version minimale de libadwaita sera 1.7 (Ubuntu 24.04 fournit 1.5).
- En palier compact, une classe `compact` porte les cibles tactiles à 44 px : boutons des cartes, actions du post ouvert, boutons d'en-tête (sauf ceux de la fenêtre). La croix de fermeture des colonnes laisse alors la place au titre ; « Fermer la colonne » reste dans le menu d'options.
- Les colonnes se composent à la main : Following et Notifications sont ouvertes au départ, et la barre latérale permet d'en ajouter. Chaque colonne a un bouton de fermeture et un menu « Options de la colonne » (déplacer vers la gauche ou la droite, fermer). On les réordonne aussi par glisser-déposer dans la barre latérale : la rangée glissée s'atténue et un trait de la couleur d'accent marque l'endroit du dépôt, au-dessus ou en dessous de la rangée survolée. Ctrl+Maj+Page précédente et Ctrl+Maj+Page suivante déplacent la colonne courante. La liste et l'ordre des colonnes sont enregistrés dans GSettings (clé `columns`) et retrouvés au lancement suivant.

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

## Connexion

Pas de maquette : l'écran suit les motifs de la HIG (icône, titre, champ, bouton en pilule).

- Écran de connexion au lancement s'il n'y a pas de session dans le trousseau : identifiant (`AdwEntryRow`), « Se connecter », puis attente pendant que le navigateur s'ouvre (« Rouvrir le navigateur », « Annuler »). Les erreurs s'affichent sous le champ et sont annoncées aux lecteurs d'écran.
- Protocole : OAuth du protocole AT avec `atrium-oauth` (`src/auth/`). Tsubame ouvre un petit serveur sur 127.0.0.1, le navigateur y revient après la connexion, puis Tsubame échange le code contre des jetons. La session (jetons, clé DPoP) va dans le trousseau avec `oo7` : Secret Service hors Flatpak, portail Secret dans Flatpak.
- L'identifiant du compte s'affiche sous le titre de la barre latérale. « Se déconnecter… » dans le menu principal demande confirmation, révoque les jetons si le serveur répond, et retire la session du trousseau.
- Les builds de développement proposent « Continuer avec les posts de démonstration », tant que les fils ne sont pas branchés sur le réseau.

Fils (`src/api.rs`) :

- Following vient de `app.bsky.feed.getTimeline`, Discover du fil « What's Hot » de Bluesky (`getFeed`). Pages de 30 posts ; la suite se charge en arrivant en bas de la colonne. « Actualiser » est dans l'en-tête de la colonne et dans son menu ⋮ (F5, Ctrl+R pour la colonne courante).
- Les cartes indiquent un repost (« X a reposté ») ou une réponse (« En réponse à X »), l'heure relative (« 4 min », « 2 h », « 3 j », puis la date) la première image (ou la vignette d'une vidéo) à ses proportions, avec la pastille « ALT » quand un texte alternatif existe et « +N » quand le post en a d'autres, et l'avatar de l'auteur (initiales en attendant). Les images sont téléchargées en arrière-plan (6 à la fois, 10 Mo au plus), décodées hors de la boucle GTK et gardées en mémoire (400 au plus) : `src/images.rs`.
- La discussion affiche le post tout de suite, puis ses réponses directes (`getPostThread`).
- Notifications et « Linux et libre » (fil fictif des maquettes) affichent « Pas encore disponible ».
- Chaque colonne a ses états : chargement, liste, liste vide, erreur avec « Réessayer ». Une session expirée ou illisible ramène à l'écran de connexion, identifiant pré-rempli, et la session est retirée du trousseau.

**Point ouvert : client OAuth de production.** Tsubame utilise pour l'instant le client de développement « localhost » du protocole AT, qui ne demande rien de publié mais n'est pas fait pour une version distribuée. Il faudra publier les métadonnées du client en https (par exemple sur GitHub Pages) et utiliser leur adresse comme `client_id`, avec une URI de redirection adaptée à une application native.

## Limites de la proposition

- Les maquettes sont statiques et n'ont pas été testées sur de vrais appareils.
- Les écrans de notifications, de listes, de starter packs et de la plupart des préférences ne sont pas dessinés en détail : ils suivent les mêmes motifs que les écrans dessinés.
- Les couleurs des maquettes sont un point de départ. Dans l'application, elles viennent de libadwaita.
