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

## Licence

GPL-3.0 ou ultérieure, voir `LICENSE`.
