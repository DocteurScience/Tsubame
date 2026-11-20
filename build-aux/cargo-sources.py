#!/usr/bin/env python3
"""Génère build-aux/cargo-sources.json à partir de Cargo.lock.

Le build Flatpak n'a pas accès au réseau : chaque crate de crates.io est
téléchargée par flatpak-builder comme une archive vérifiée par sa somme
SHA-256 (celle de Cargo.lock), puis Cargo lit ces crates depuis
$CARGO_HOME/vendor au lieu de crates.io.

À relancer après chaque modification de Cargo.lock :

    python3 build-aux/cargo-sources.py

Seules les dépendances publiées sur crates.io sont prises en charge.
"""

import json
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
VENDOR_DIR = "cargo/vendor"

CARGO_CONFIG = f"""\
[source.vendored-sources]
directory = "{VENDOR_DIR}"

[source.crates-io]
replace-with = "vendored-sources"
"""


def main() -> int:
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
    sources = []
    for package in lock["package"]:
        source = package.get("source")
        if source is None:
            continue  # Le paquet du projet lui-même.
        if source != CRATES_IO:
            print(f"Source non prise en charge pour {package['name']} : {source}", file=sys.stderr)
            return 1
        name, version, checksum = package["name"], package["version"], package["checksum"]
        dest = f"{VENDOR_DIR}/{name}-{version}"
        sources.append(
            {
                "type": "archive",
                "archive-type": "tar-gzip",
                "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
                "sha256": checksum,
                "dest": dest,
            }
        )
        # Cargo exige ce fichier dans chaque crate d'un dossier vendored.
        sources.append(
            {
                "type": "inline",
                "contents": json.dumps({"package": checksum, "files": {}}),
                "dest": dest,
                "dest-filename": ".cargo-checksum.json",
            }
        )
    sources.append(
        {
            "type": "inline",
            "contents": CARGO_CONFIG,
            "dest": "cargo",
            "dest-filename": "config.toml",
        }
    )
    out = ROOT / "build-aux" / "cargo-sources.json"
    out.write_text(json.dumps(sources, indent=4) + "\n")
    print(f"{out.relative_to(ROOT)} : {len(sources)} sources")
    return 0


if __name__ == "__main__":
    sys.exit(main())
