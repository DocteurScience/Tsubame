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
- Authentification : OAuth (crate `atrium-oauth`). Les jetons vont dans le trousseau système (libsecret).
- Protocole AT : famille `atrium` (`atrium-api`, `bsky-sdk`, `atrium-oauth`), licence MIT, couvre `app.bsky.*` et `chat.bsky.*`.
- Internationalisation : prévue dès le départ avec gettext (`gettext-rs`). Le français est la langue source des chaînes.
- Distribution : Flatpak, build Meson qui appelle Cargo.
- Colonnes sur grand écran : composition manuelle par l'utilisateur, avec Following et Notifications ouverts par défaut.
- Textes d'interface : vouvoiement.

## Principes d'interface

- Suivre la GNOME HIG (https://developer.gnome.org/hig/) et utiliser les widgets libadwaita plutôt que des composants maison : `AdwNavigationSplitView`, `AdwOverlaySplitView`, `AdwViewSwitcherBar`, `AdwDialog`, `AdwPreferencesDialog`, `AdwToggleGroup`, `AdwBreakpoint`, `AdwAboutDialog`.
- Les maquettes (`docs/mockups/source/*.dc.html`) utilisent des couleurs en dur pour être autonomes. Dans l'application, ne rien coder en dur : utiliser les couleurs nommées de libadwaita (accent, fond de fenêtre, fond de carte) et les polices du système. Le thème clair et sombre et la couleur d'accent suivent le système.
- Accessibilité : tout bouton qui n'a qu'une icône porte un nom accessible. Cibles tactiles d'au moins 44 px en mobile. Navigation complète au clavier. Texte alternatif des images affiché et modifiable.
- Textes d'interface : phrases courtes, voir la section « Writing Style » de la HIG. Vouvoiement.

## Marque et ressources graphiques

- Ne pas reprendre le nom, le logo ni les illustrations de Bluesky. Le dépôt `bluesky-social/social-app` est sous licence MIT, mais ses images, icônes, polices et marques ne le sont pas (voir son `ASSETS.md`).
- L'icône de l'application est dans `data/icons/hicolor/`. Elle vient d'une image fournie par l'utilisateur, vectorisée. Ne pas ajouter l'image source au dépôt sans confirmation de ses droits.

## Conventions proposées (à valider par l'utilisateur)

- Format avec `cargo fmt`, lints avec `cargo clippy`.
- Les jetons et mots de passe vont dans le trousseau système (libsecret), jamais dans un fichier.

## Suivi des changements

- Chaque modification suit le skill `changement` (`.claude/skills/changement/SKILL.md`) : vérifications, traduction, accessibilité, documentation, `CHANGELOG.md`.
- Une publication suit le skill `nouvelle-version` (`.claude/skills/nouvelle-version/SKILL.md`).
- `CHANGELOG.md` résume les évolutions visibles ; `docs/design.md` décrit l'état de chaque fonction.
