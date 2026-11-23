# Tsubame
Client Bluesky natif pour GNOME, en Rust avec GTK 4 et libadwaita. En cours de développement : l'interface affiche pour l'instant de faux posts, sans connexion au réseau.

## Compiler

Dépendances : Rust 1.95 ou plus, GTK 4.14 ou plus, libadwaita 1.5 ou plus, gettext. Sur Debian ou Ubuntu : `libgtk-4-dev libadwaita-1-dev libssl-dev gettext`. La connexion garde la session dans le trousseau du système (GNOME Trousseau ou tout service Secret Service).

Pour lancer l'application pendant le développement :

```bash
cargo run
```

Pour l'installer avec ses icônes, son fichier `.desktop` et ses métadonnées :

```bash
meson setup _build --prefix=/usr
meson compile -C _build
meson install -C _build
```

Avec `cargo run`, les réglages (GSettings) utilisent le schéma compilé dans le dossier de build. Une fois l'application installée, ils utilisent le schéma installé dans `glib-2.0/schemas`.

`meson test -C _build` valide le schéma GSettings, le fichier `.desktop` et les métadonnées AppStream.

Vérification d'accessibilité, facultative : elle lance Tsubame en démonstration sous Xvfb et contrôle, comme Orca les voit, les noms des éléments, l'accès au clavier et la taille des cibles en palier mobile. Elle demande `xvfb-run`, `xdotool`, `dbus-run-session`, `at-spi2-core` et Python avec `pyatspi` (sinon le test est ignoré) :

```bash
meson configure _build -Da11y-tests=true
```

```bash
meson test -C _build --suite a11y
```

## Flatpak

Le manifeste est dans `build-aux/`. Il utilise le runtime GNOME 51 et l'extension `rust-stable`, à installer depuis Flathub :

```bash
flatpak install flathub org.gnome.Sdk//51 org.freedesktop.Sdk.Extension.rust-stable//26.08
```

```bash
flatpak-builder --user --install --force-clean build-dir build-aux/io.github.DocteurScience.Tsubame.json
```

Le build se fait hors ligne : les crates viennent de `build-aux/cargo-sources.json`. Après chaque modification de `Cargo.lock`, régénérer ce fichier :

```bash
python3 build-aux/cargo-sources.py
```

GitHub Actions construit le paquet à chaque push (workflow `Flatpak`) et le met à disposition comme artefact `tsubame.flatpak`.

## Snap

La recette est `snap/snapcraft.yaml` : base core24 et extension `gnome` (GTK 4.14, libadwaita 1.5). Construction locale :

```bash
snapcraft
```

```bash
sudo snap install --dangerous tsubame_0.1.0_amd64.snap
```

La session Bluesky est gardée dans le trousseau, auquel un Snap n'accède qu'après cette connexion :

```bash
sudo snap connect tsubame:password-manager-service
```

GitHub Actions construit aussi le Snap à chaque push (workflow `Snap`, artefact `tsubame-snap`).

## Licence

GPL-3.0 ou ultérieure, voir `LICENSE`.
