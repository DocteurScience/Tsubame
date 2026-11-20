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
