# Tsubame

Client Bluesky natif pour le bureau GNOME. Application Rust avec GTK 4 et libadwaita. Licence GPL-3.0 (voir `LICENSE`).

Lire d'abord `docs/design.md` : il décrit les écrans, les widgets à utiliser, les paliers d'adaptation et les points encore ouverts.

## Décisions prises

- Nom : Tsubame (燕, l'hirondelle). Identifiant d'application proposé : `io.github.DocteurScience.Tsubame`.
- Langage : Rust, avec `gtk4-rs` et `libadwaita-rs`.
- Interface en français.
- Les posts s'affichent en cartes (une carte par post), comme dans Tegami Mail (https://github.com/DocteurScience/Tegami_Mail). Un volet de lecture affiche la discussion.
- Sur grand écran, plusieurs colonnes de cartes côte à côte plutôt qu'une colonne étirée.
- Objectif fonctionnel : couvrir toutes les fonctions de Bluesky (voir la table de couverture dans `docs/design.md`).

## À confirmer avec l'utilisateur avant de coder

- Authentification : mot de passe d'application ou OAuth.
- Bibliothèque pour le protocole AT en Rust : évaluer l'état actuel des crates existantes (maintenance, couverture des lexicons `app.bsky.*`, messagerie) avant de choisir.
- Internationalisation : prévoir ou non d'autres langues que le français.
- Distribution : Flatpak envisagé, à confirmer.
- Le nombre de colonnes sur grand écran s'ouvre-t-il automatiquement ou se compose-t-il à la main ? Proposition : composition manuelle, avec Following et Notifications ouverts par défaut.

## Principes d'interface

- Suivre la GNOME HIG (https://developer.gnome.org/hig/) et utiliser les widgets libadwaita plutôt que des composants maison : `AdwNavigationSplitView`, `AdwOverlaySplitView`, `AdwViewSwitcherBar`, `AdwDialog`, `AdwPreferencesDialog`, `AdwToggleGroup`, `AdwBreakpoint`, `AdwAboutDialog`.
- Les maquettes (`docs/mockups/source/*.dc.html`) utilisent des couleurs en dur pour être autonomes. Dans l'application, ne rien coder en dur : utiliser les couleurs nommées de libadwaita (accent, fond de fenêtre, fond de carte) et les polices du système. Le thème clair et sombre et la couleur d'accent suivent le système.
- Accessibilité : tout bouton qui n'a qu'une icône porte un nom accessible. Cibles tactiles d'au moins 44 px en mobile. Navigation complète au clavier. Texte alternatif des images affiché et modifiable.
- Textes d'interface : phrases courtes, voir la section « Writing Style » de la HIG. Le choix entre tutoiement et vouvoiement reste à faire.

## Marque et ressources graphiques

- Ne pas reprendre le nom, le logo ni les illustrations de Bluesky. Le dépôt `bluesky-social/social-app` est sous licence MIT, mais ses images, icônes, polices et marques ne le sont pas (voir son `ASSETS.md`).
- L'icône de l'application est dans `data/icons/hicolor/`. Elle vient d'une image fournie par l'utilisateur, vectorisée. Ne pas ajouter l'image source au dépôt sans confirmation de ses droits.

## Conventions proposées (à valider par l'utilisateur)

- Format avec `cargo fmt`, lints avec `cargo clippy`.
- Les jetons et mots de passe vont dans le trousseau système (libsecret), jamais dans un fichier.
