# Journal des changements

Les évolutions de Tsubame visibles par les personnes qui l'utilisent ou le
distribuent, de la plus récente à la plus ancienne. Le détail est dans
l'historique git ; l'état de chaque fonction, dans `docs/design.md`.

Chaque changement ajoute une ligne sous « Non publié ». À la publication d'une
version, cette section prend le numéro et la date de la version, et son
résumé va dans les notes de version de
`data/io.github.DocteurScience.Tsubame.metainfo.xml.in` (affichées par GNOME
Logiciels et Flathub).

## Non publié

### Écrire

- Émojis, coller une image (Ctrl+V) ou la déposer sur la boîte de rédaction.
- Étiquette de contenu sur les images (nudité, contenu sexuel, violence).
- Images dans les posts suivants d'un fil.
- Publier une vidéo ou un GIF animé (100 Mo au plus), avec son texte alternatif.

### Lire

- Messages : actualisation automatique, messages plus anciens, nouvelle conversation, réactions, bouton « Envoyer un message » sur les profils.
- Listes : créer, modifier, supprimer, ajouter ou retirer des comptes ; masquer ou bloquer les comptes d'une liste de modération.
- Starter packs dans Explorer : voir leurs comptes et leurs flux, tout suivre.
- Recherche de flux dans Explorer.
- Liens, mentions et mots-dièse cliquables dans les posts ; page d'un mot-dièse.
- Page « Flux » : épingler, réordonner, retirer ses flux ; découvrir et
  rechercher des flux, les ouvrir, les ajouter.

### Intégration continue

- Durée maximale des jobs : un blocage échoue au bout d'une heure au lieu de
  tourner six heures.

## 0.1.0 (en préparation)

Première version.

### Lire

- Colonnes de cartes côte à côte (Following, Discover, notifications, flux
  épinglés), composées par l'utilisateur et gardées d'une session à l'autre ;
  une colonne en mobile, jusqu'à 360 px.
- Volet de discussion à droite, en dessous ou masqué, ou en page.
- Posts cités, cartes de liens, grille d'images et visionneuse avec texte
  alternatif, vidéos lues dans la carte.
- Étiquettes de contenu : avertissement flouté ou post masqué, selon les
  préférences.
- Explorer et recherche, signets, listes, profils (abonnés, abonnements,
  qui a aimé ou reposté).
- Notifications : filtre Toutes ou Mentions, regroupement (« Pierre et
  4 autres ont aimé votre post »), notifications du bureau.
- Messagerie privée.

### Écrire

- Posts avec images et texte alternatif, réponses, citations, fils de
  plusieurs posts, brouillons.
- Suggestions de mentions, aperçu des liens, choix de qui peut répondre et
  citer.
- Modifier son profil, épingler ou supprimer ses posts.

### Modération

- Masquer, bloquer, signaler un post ou un compte ; mots masqués ; réglages
  des étiquettes.

### Compte

- Connexion OAuth dans le navigateur ; Tsubame ne voit pas le mot de passe.
  Session gardée dans le trousseau.
- Client OAuth publié pour les paquets
  (`https://docteurscience.github.io/Tsubame/oauth/client-metadata.json`).

### Accessibilité et langues

- Noms accessibles, navigation au clavier, cibles de 44 px en mobile,
  vérifiés automatiquement par AT-SPI.
- Interface en français, traduction anglaise complète.

### Paquets

- Flatpak (runtime GNOME 51) et Snap (core24, extension GNOME), construits
  par GitHub Actions sur `main`, les tags `v*` et à la demande.
