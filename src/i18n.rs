//! Traduction des textes d'interface avec gettext.
//!
//! Le français est la langue source : les chaînes sont écrites en français
//! dans le code et traduites vers les autres langues dans `po/`.

use gettextrs::{bind_textdomain_codeset, bindtextdomain, textdomain};

pub const GETTEXT_PACKAGE: &str = "tsubame";

/// Dossier des traductions compilées. Meson le fournit au moment du build.
const LOCALEDIR: &str = match option_env!("TSUBAME_LOCALEDIR") {
    Some(dir) => dir,
    None => "/usr/share/locale",
};

/// À appeler avant de créer l'application. La locale elle-même est réglée
/// par GTK à son initialisation.
pub fn init() {
    // Sans traduction disponible, gettext renvoie la chaîne source : on peut
    // ignorer les erreurs ici.
    let _ = bindtextdomain(GETTEXT_PACKAGE, LOCALEDIR);
    let _ = bind_textdomain_codeset(GETTEXT_PACKAGE, "UTF-8");
    let _ = textdomain(GETTEXT_PACKAGE);
}

#[cfg(test)]
mod tests {
    /// xgettext lit le Rust comme du C : une chaîne coupée par « \ » en fin
    /// de ligne y garderait l'indentation de la ligne suivante, et sa
    /// traduction ne serait jamais trouvée. Les chaînes traduites tiennent
    /// donc sur une ligne.
    #[test]
    fn translated_strings_are_not_split_across_lines() {
        let pattern = regex::Regex::new(
            r#"\bn?gettext\(\s*(?:"(?:[^"\\]|\\.)*"\s*,\s*)*"(?:[^"\\]|\\.)*\\\n"#,
        )
        .unwrap();
        let mut offenders = Vec::new();
        let mut dirs = vec![std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src"
        ))];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    if let Some(found) = pattern.find(&text) {
                        offenders.push(format!(
                            "{}: {}",
                            path.display(),
                            &found.as_str()[..40.min(found.len())]
                        ));
                    }
                }
            }
        }
        assert!(offenders.is_empty(), "chaînes coupées : {offenders:?}");
    }

    /// xgettext ne trouve que les chaînes littérales : `gettext(variable)`
    /// ne serait jamais traduit. Seuls les faux posts de démonstration
    /// (`data.rs`) passent par une variable, déjà extraite ailleurs.
    #[test]
    fn translated_strings_are_literals() {
        let pattern = regex::Regex::new(r#"\bn?gettext\(\s*[^"\s)]"#).unwrap();
        let mut offenders = Vec::new();
        let mut dirs = vec![std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src"
        ))];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && !path.ends_with("data.rs")
                    && !path.ends_with("i18n.rs")
                {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for found in pattern.find_iter(&text) {
                        offenders.push(format!("{}: {}", path.display(), found.as_str()));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "chaînes non littérales : {offenders:?}"
        );
    }
}
