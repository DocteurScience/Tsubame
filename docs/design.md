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
- Notifications (`listNotifications`, puis `getPosts` pour les posts concernés) : comme dans la maquette `Colonnes`, une ligne de contexte (« a aimé votre post », « a répondu à votre post », « vous suit »…) au-dessus du nom de la personne, puis le post concerné ; l'ouvrir ouvre sa discussion. Les notifications pas encore vues sont teintées de la couleur d'accent et comptées dans la pastille de la barre latérale et de l'onglet compact ; en ouvrir une les marque toutes comme vues (`updateSeen`). Les notifications d'un même post ne sont pas encore regroupées.
- J'aime et Reposter, sur les cartes et dans la discussion : l'affichage change tout de suite (couleur d'accent, compteur, « Aimé · 49 »), puis Bluesky confirme (`com.atproto.repo.createRecord`, `deleteRecord`). En cas d'échec, l'état revient en arrière et un message (`AdwToast`) l'explique. Toutes les cartes d'un même post partagent son état (`src/interactions.rs`). Les boutons annoncent leur état « enfoncé » aux lecteurs d'écran. Répondre ouvre la boîte de rédaction.
- Boîte de rédaction (`src/compose.rs`, maquette `Compose`) : `AdwDialog` « Nouveau post » ou « Répondre » (avec le post auquel on répond en rappel), Annuler et Publier dans l'en-tête, langue du post et compteur « n / 300 » en bas (300 graphèmes, en couleur d'erreur au-delà). Liens, mots-dièse et mentions deviennent des facettes à la publication (`src/richtext.rs`), les mentions après résolution de l'identifiant. Fermer avec un texte en cours demande confirmation. Accès : bouton « Nouveau post » de la barre latérale, bouton flottant en palier compact (maquette `Mobile`), Ctrl+N, « Répondre » sur les cartes et dans la discussion ; Ctrl+Entrée publie. Images : jusqu'à 4, choisies avec le sélecteur de fichiers du système (portail en Flatpak). Vignettes avec « Retirer l'image » et « + ALT » / « ALT ✓ » comme dans la maquette ; le texte alternatif s'écrit dans une boîte dédiée (image en grand, 2000 caractères au plus). À la publication, chaque image est redressée, réduite à 2000 px de côté et réencodée sous la limite de 1 Mo de Bluesky, ce qui retire ses métadonnées (position GPS comprise) : `src/photo.rs`. Un post peut n'avoir que des images. Pas encore : vidéo, GIF, émojis, coller ou glisser une image, étiquettes, réglages de réponse et de citation, brouillons, fils de plusieurs posts, suggestions de mentions.
- Profils (`src/profile.rs`, maquette `Profil`) : le nom et l'avatar d'un auteur, sur les cartes et dans la discussion, ouvrent son profil, empilé dans la navigation avec retour. En-tête : bannière, avatar, nom, identifiant (« · vous suit »), bio, compteurs au format français (« 1 284 abonnés ») ; « abonnés » et « abonnements » ouvrent la liste des comptes (`getFollowers`, `getFollows`, par 50, suite au défilement) avec « Suivre » sur chaque ligne, « Suivre » / « Ne plus suivre ». Onglets Posts, Réponses, Médias, et J'aime sur son propre profil (l'API ne donne pas ceux des autres) ; boutons liés en attendant `AdwToggleGroup`. Un post du profil ouvre sa discussion en page empilée. « Profil » dans la barre latérale montre le sien, avec « Modifier le profil » (`src/edit_profile.rs`) : `AdwDialog` avec bannière et avatar (« Changer » par le sélecteur de fichiers, « Retirer »), nom affiché (64 caractères) et bio (256), compteurs « n / max ». Les images sont préparées comme celles des posts (redressées, réduites, métadonnées retirées). L'enregistrement relit `app.bsky.actor.profile/self`, ne change que ces quatre champs, garde les autres (post épinglé…) et réécrit le profil à condition qu'il n'ait pas changé entre-temps (`swapRecord`). Menu ⋮ de l'en-tête (sauf sur son propre profil) : « Copier le lien », « Masquer » / « Ne plus masquer », « Bloquer… » (après confirmation) / « Débloquer », « Signaler… ». Un compte bloqué n'affiche plus ses posts, et « Suivre » est grisé. Pas encore : notifications d'activité, message, suivis communs, onglets Vidéos, Flux, Listes et Starter packs.
- Explorer (`src/explore.rs`, maquette `Explorer`) : champ de recherche dans l'en-tête (Ctrl+K ou Ctrl+F depuis n'importe où). Sans recherche : « Sujets du moment » (`getTrendingTopics`, un clic lance la recherche) et « Personnes à suivre » (`getSuggestions`). Avec une recherche : onglets Posts (`searchPosts`, en cartes, pagination au défilement) et Personnes (`searchActors`). Les rangées de personnes ouvrent le profil et portent « Suivre » / « Abonné ». Pas encore : onglets Tendances, Flux et Starter packs de la maquette, recherche de flux.
- Fils épinglés : une fois connecté, « Ajouter une colonne » propose, après Following, Notifications et Discover, les fils personnalisés épinglés sur Bluesky (`getPreferences`, `savedFeedsPrefV2` ou l'ancien `savedFeedsPref`, puis `getFeedGenerators` pour leurs noms). Une colonne de fil est enregistrée sous la forme `feed:<adresse at://>` et se rouvre au lancement. « Linux et libre », fil fictif des maquettes, n'est proposé qu'en démonstration. Pas encore : listes épinglées, fils enregistrés mais non épinglés.
- Signets : bouton signet sur les cartes et dans la discussion (sans compteur : un signet est privé), état partagé comme J'aime et Reposter (`createBookmark`, `deleteBookmark`, jamais une suppression d'enregistrement). Page « Signets » de la barre latérale (`getBookmarks`).
- Listes (`src/feed_page.rs`) : page « Listes » avec mes listes (`getLists`), leur nombre de membres et leur type. Une liste de curation ouvre son fil (`getListFeed`), une liste de modération ses membres (`getList`), en page empilée. Pas encore : créer ou modifier une liste, listes des autres comptes.
- Préférences de modération (`src/preferences.rs`, `src/moderation.rs`, maquette `Reglages`) : `AdwPreferencesDialog` ouvert par « Préférences » du menu principal ou Ctrl+virgule. Groupe « Listes et comptes » avec le nombre de comptes masqués (`getMutes`), bloqués (`getBlocks`) et de mots masqués ; chacun ouvre une sous-page pour débloquer, ne plus masquer, ajouter ou retirer un mot. Groupe « Contenu sensible » : contenu pour adultes et, par étiquette, Afficher, Avertir ou Masquer. Groupe « Services d'étiquetage » : le service par défaut et ceux auxquels le compte est abonné. Chaque changement réécrit les préférences du compte (`getPreferences` puis `putPreferences`), en gardant les entrées que Tsubame ne connaît pas. Les posts qui contiennent un mot masqué sont retirés des fils (sans tenir compte de la casse). Les étiquettes du post et de son auteur (sauf celles annulées) suivent ces réglages : « Masquer » retire le post des fils, « Avertir » floute ses images sous le nom de l'étiquette et un bouton « Afficher » (badges ALT et +N cachés jusque-là). Les étiquettes « !hide » et « !warn » de la modération Bluesky ne se règlent pas ; sans contenu pour adultes, ses étiquettes masquent le post. Un post étiqueté de démonstration (Inès Garnier) montre l'avertissement. Menu ⋮ des cartes : « Copier le lien » (lien web du post) ; sur mes posts (reconnus au DID de leur adresse), « Épingler sur le profil » / « Retirer du haut du profil » (champ `pinnedPost` du profil, réécrit comme pour « Modifier le profil ») et « Supprimer le post… » (après confirmation) ; le post épinglé s'affiche en tête du profil avec « Épinglé » ; sur ceux des autres, « Masquer @… » et « Bloquer @… » (après confirmation), « Signaler le post… » et « Signaler @… ». Le signalement (`src/report.rs`) ouvre un `AdwDialog` : un motif parmi six (spam, contenu trompeur, contenu sexuel non étiqueté, comportement hostile, infraction aux règles, autre), des précisions facultatives (2000 caractères au plus), Envoyer. Il part au service de modération de Bluesky (`createReport`, `atproto-proxy: did:plc:ar7c4by46qjdydhdevvrndac#atproto_labeler`). Pas encore : étiquettes propres aux services tiers, avertissement sur le texte seul ou sur un profil, masquer par mot-clic seul ou pour une durée, signaler à un service tiers, signaler depuis un message, listes de modération auxquelles on s'abonne.
- Messagerie (`src/chat.rs`, `src/messages.rs`, maquette `Messages`) : deux volets (`AdwNavigationSplitView`, un seul aux paliers étroits). À gauche, onglets Conversations et « Demandes · n », chaque conversation avec avatar, nom (ou nom du groupe), heure, dernier message et pastille de non-lus. À droite, les bulles (les miennes à droite, en couleur d'accent), séparées par jour, et « Écrire un message… » avec Envoyer (Entrée). Ouvrir une conversation la marque lue (`updateRead`) ; une demande s'accepte depuis un bandeau (`acceptConvo`). La pastille de « Messages » compte les non-lus. Les requêtes passent par le service de chat de Bluesky (`atproto-proxy: did:web:api.bsky.chat#bsky_chat`), avec la portée OAuth `transition:chat.bsky`. Pas encore : nouvelle conversation, réactions, partage de post, réglages, rafraîchissement automatique (bouton Actualiser en attendant), messages plus anciens.
- Chaque colonne a ses états : chargement, liste, liste vide, erreur avec « Réessayer ». Une session expirée ou illisible ramène à l'écran de connexion, identifiant pré-rempli, et la session est retirée du trousseau.

**Point ouvert : client OAuth de production.** Tsubame utilise pour l'instant le client de développement « localhost » du protocole AT, qui ne demande rien de publié mais n'est pas fait pour une version distribuée. Il faudra publier les métadonnées du client en https (par exemple sur GitHub Pages) et utiliser leur adresse comme `client_id`, avec une URI de redirection adaptée à une application native.

## Limites de la proposition

- Les maquettes sont statiques et n'ont pas été testées sur de vrais appareils.
- Les écrans de notifications, de listes, de starter packs et de la plupart des préférences ne sont pas dessinés en détail : ils suivent les mêmes motifs que les écrans dessinés.
- Les couleurs des maquettes sont un point de départ. Dans l'application, elles viennent de libadwaita.
