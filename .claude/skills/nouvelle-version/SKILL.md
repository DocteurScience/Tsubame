---
name: nouvelle-version
description: Procédure pour publier une nouvelle version de Tsubame - numéro de version, journal des changements, notes de version AppStream, traduction, tag, paquets Flatpak et Snap. À utiliser quand on prépare ou publie une version.
---

# Publier une version

Le numéro suit `MAJEUR.MINEUR.CORRECTIF`. Avant 1.0, une version qui ajoute
des fonctions augmente MINEUR ; une version de corrections, CORRECTIF.

## 1. Numéro de version

Le mettre à jour aux trois endroits :

- `Cargo.toml` (`version`) ; `Cargo.lock` suit au prochain `cargo build` ;
- `meson.build` (`project(… version: …)`) ;
- `snap/snapcraft.yaml` (`version`).

Puis régénérer les sources hors ligne du Flatpak, puisque `Cargo.lock` a changé :

```bash
cargo build && python3 build-aux/cargo-sources.py
```

## 2. Journal des changements

Dans `CHANGELOG.md`, la section « Non publié » devient
`## X.Y.Z (AAAA-MM-JJ)`. Ajouter au-dessus une nouvelle section
« Non publié », vide.

## 3. Notes de version

Dans `data/io.github.DocteurScience.Tsubame.metainfo.xml.in`, ajouter en tête
de `<releases>` :

```xml
<release version="X.Y.Z" date="AAAA-MM-JJ" type="stable">
  <description>
    <p>Une phrase de résumé.</p>
    <ul>
      <li>Les changements principaux, tirés du journal</li>
    </ul>
  </description>
</release>
```

`type="development"` tant que Tsubame n'est pas prêt pour tout le monde.
Phrases courtes, du point de vue de la personne qui utilise l'application.
Ces textes sont affichés par GNOME Logiciels et Flathub, et ils se traduisent :
mettre à jour `po/en.po` comme dans le skill `changement`.

```bash
appstreamcli validate --no-net data/io.github.DocteurScience.Tsubame.metainfo.xml.in
```

## 4. Vérifier et fusionner

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

```bash
meson setup --wipe _build -Da11y-tests=true && ninja -C _build && meson test -C _build
```

Committer (« Version X.Y.Z »), ouvrir la pull request vers `main`, attendre
le workflow « Vérifications », fusionner.

## 5. Tag et paquets

Sur `main` à jour :

```bash
git tag -a vX.Y.Z -m "Tsubame X.Y.Z" && git push origin vX.Y.Z
```

Le tag lance les workflows « Flatpak » et « Snap ». Leurs artefacts
(`tsubame-flatpak`, `tsubame-snap`) sont les paquets de la version ; les
joindre à une release GitHub si on en publie une. Un échec sur
`apt update` ou au démarrage de LXD vient du runner : relancer le job.

## 6. Avant de publier dans les boutiques

- Le fichier du client OAuth doit être en ligne et inchangé :
  `https://docteurscience.github.io/Tsubame/oauth/client-metadata.json`
  (GitHub Pages, `docs/` de `main`). S'il change d'adresse, les connexions
  des versions déjà installées échouent.
- Tester l'installation et la connexion avec les deux paquets sur un vrai
  bureau.
- Flathub : la première fois, soumettre le manifeste au dépôt
  `flathub/flathub` ; ensuite, mettre à jour le dépôt de l'application sur
  Flathub (version, sources). Flathub compose lui-même les métadonnées
  AppStream.
- Snap Store : enregistrer le nom `tsubame` une fois, puis
  `snapcraft upload --release=edge tsubame_X.Y.Z_amd64.snap`, et passer en
  `stable` après essai. Ces deux étapes demandent les comptes du mainteneur.
