//! Réglages enregistrés avec GSettings.

use gtk::gio;

/// Réglages de l'application. Utilise le schéma installé s'il existe, sinon
/// celui compilé par `build.rs` (lancement avec `cargo run`).
pub fn settings() -> gio::Settings {
    let default = gio::SettingsSchemaSource::default();
    if default
        .as_ref()
        .and_then(|source| source.lookup(crate::APP_ID, true))
        .is_some()
    {
        return gio::Settings::new(crate::APP_ID);
    }
    let source = gio::SettingsSchemaSource::from_directory(
        env!("TSUBAME_DEV_SCHEMA_DIR"),
        default.as_ref(),
        false,
    )
    .expect("schéma GSettings de développement illisible");
    let schema = source
        .lookup(crate::APP_ID, false)
        .expect("schéma GSettings introuvable");
    gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None)
}
