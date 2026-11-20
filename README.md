# Tsubame
Client Bluesky natif pour GNOME, en Rust avec GTK 4 et libadwaita. En cours de développement : l'interface affiche pour l'instant de faux posts, sans connexion au réseau.

## Compiler

Dépendances : Rust 1.92 ou plus, GTK 4.14 ou plus, libadwaita 1.5 ou plus, gettext. Sur Debian ou Ubuntu : `libgtk-4-dev libadwaita-1-dev gettext`.

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

`meson test -C _build` valide le fichier `.desktop` et les métadonnées AppStream.

## Licence

GPL-3.0 ou ultérieure, voir `LICENSE`.
