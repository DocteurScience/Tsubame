---
name: changement
description: Routine à suivre pour chaque modification de Tsubame (fonction, correction, interface, dépendance) - vérifications, traduction, accessibilité, documentation, journal des changements, commit. À utiliser avant de committer un changement du code ou des données.
---

# Routine d'un changement

Tsubame est un client Bluesky en Rust, GTK 4.14 et libadwaita 1.5. Lire d'abord
`CLAUDE.md` (décisions, principes d'interface) et `docs/design.md` (état de
chaque fonction). Suivre ces étapes pour chaque changement, dans l'ordre.

## 1. Code

- Pas de couleurs en dur : couleurs nommées de libadwaita dans `data/style.css`.
- Textes d'interface en français, vouvoiement, phrases courtes (HIG, Writing Style).
- Préférer les widgets libadwaita aux composants maison.

## 2. Traduction (gettext)

- Toute chaîne visible passe par `gettext`, `ngettext` ou `pgettext`.
- Chaque chaîne traduite est **littérale** et tient **sur une ligne** :
  xgettext lit le Rust comme du C. Une chaîne coupée par `\` ou passée en
  variable ne serait jamais traduite. Les tests de `src/i18n.rs` le vérifient.
- Le nombre d'un compteur se place dans la chaîne (`ngettext("{} abonné", "{} abonnés", n)`).
- Un mot qui change de sens selon l'endroit prend un contexte :
  `pgettext("masquer un compte", "Masquer")`.
- Un caractère spécial s'écrit tel quel dans la chaîne, jamais en `\u{…}`.
- Nouveau fichier source qui traduit : l'ajouter à `po/POTFILES.in`, dans l'ordre alphabétique.
- Mettre à jour la traduction anglaise :

```bash
meson setup _build && ninja -C _build tsubame-pot
```

```bash
msgmerge -U --backup=none po/en.po po/tsubame.pot
```

  Traduire les entrées vides ou `fuzzy` de `po/en.po`, garder les mêmes
  `{}`, `{name}`, `{n}`, puis vérifier :

```bash
msgfmt --check --statistics -o /dev/null po/en.po
```

  Seule `translator-credits` reste vide. `po/tsubame.pot` n'est pas versionné.

## 3. Accessibilité

- Tout bouton qui n'a qu'une icône porte un nom accessible
  (`update_property(&[gtk::accessible::Property::Label(…)])`) et une infobulle.
- Pas d'`AdwButtonContent` quand le nom accessible diffère du texte visible :
  il impose son étiquette (`labelled-by`). Utiliser `card::icon_label`.
- Icône purement décorative : `card::decorative_icon`, ou rôle `Presentation`.
- Une `GtkPicture` seule porte son texte alternatif comme nom
  (`alternative-text` ne donne qu'une description).
- Cibles de 44 px en palier mobile (`.compact` dans `data/style.css`). GTK
  compte `min-width` sans le rembourrage.
- Vérification complète (Xvfb, AT-SPI, xdotool, pyatspi) :

```bash
cargo build && build-aux/a11y-check.sh target/debug/tsubame
```

## 4. Vérifications

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
```

```bash
cargo test
```

```bash
meson test -C _build
```

Pour un changement visible, regarder l'interface en démonstration (bouton
« Continuer avec les posts de démonstration » d'un build de développement), à
1400 px et à 360 px de large.

## 5. Dépendances

Si `Cargo.lock` change, régénérer les sources hors ligne du Flatpak :

```bash
python3 build-aux/cargo-sources.py
```

## 6. Documentation

- `docs/design.md` : décrire la fonction là où elle est couverte, avec ce qui
  n'est « pas encore » fait. Mettre à jour les points ouverts.
- `README.md` : seulement si la façon de compiler, d'installer ou de
  configurer change.
- `CHANGELOG.md` : une ligne sous « Non publié », dans la bonne rubrique,
  pour tout changement visible par les personnes qui utilisent ou
  distribuent Tsubame (pas pour un simple remaniement interne).

## 7. Commit

- Un petit commit par changement, qui compile et passe les tests.
- Message en français : titre court, puis le pourquoi.
- Travailler sur la branche de développement, jamais directement sur `main`.
  Les constructions Flatpak et Snap ne tournent que sur `main`, les tags `v*`
  et à la demande ; à chaque push, seul le workflow « Vérifications » tourne.
