//! Brouillons de posts, gardés sur cet ordinateur dans
//! `~/.local/share/tsubame/drafts.json` (ou le dossier de données du
//! Flatpak). Seuls le texte et la langue sont gardés, pas les images.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Nombre de brouillons gardés ; les plus anciens partent au-delà.
const MAX_DRAFTS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    /// Identifiant : l'heure de la première sauvegarde (RFC 3339).
    pub id: String,
    pub text: String,
    /// Posts suivants du fil, le cas échéant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub more: Vec<String>,
    pub lang: String,
    /// Heure de la dernière sauvegarde (RFC 3339).
    pub saved: String,
}

fn path() -> PathBuf {
    gtk::glib::user_data_dir()
        .join("tsubame")
        .join("drafts.json")
}

/// Brouillons, du plus récent au plus ancien. Un fichier absent ou
/// illisible donne une liste vide.
pub fn load() -> Vec<Draft> {
    load_from(&path())
}

pub fn load_from(path: &Path) -> Vec<Draft> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Écrit la liste d'un coup (fichier temporaire puis renommage), pour ne
/// jamais laisser un fichier à moitié écrit.
fn store_to(path: &Path, drafts: &[Draft]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(drafts)?)?;
    std::fs::rename(&temporary, path)
}

/// Ajoute ou remplace (même identifiant) un brouillon, en tête de liste.
pub fn put(draft: Draft) -> std::io::Result<()> {
    put_in(&path(), draft)
}

pub fn put_in(path: &Path, draft: Draft) -> std::io::Result<()> {
    let mut drafts = load_from(path);
    drafts.retain(|d| d.id != draft.id);
    drafts.insert(0, draft);
    drafts.truncate(MAX_DRAFTS);
    store_to(path, &drafts)
}

pub fn remove(id: &str) -> std::io::Result<()> {
    remove_in(&path(), id)
}

pub fn remove_in(path: &Path, id: &str) -> std::io::Result<()> {
    let mut drafts = load_from(path);
    let before = drafts.len();
    drafts.retain(|d| d.id != id);
    if drafts.len() == before {
        return Ok(());
    }
    store_to(path, &drafts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(id: &str, text: &str) -> Draft {
        Draft {
            id: id.into(),
            text: text.into(),
            more: Vec::new(),
            lang: "fr".into(),
            saved: id.into(),
        }
    }

    #[test]
    fn drafts_are_kept_newest_first() {
        let dir = std::env::temp_dir().join(format!("tsubame-drafts-{}", std::process::id()));
        let path = dir.join("drafts.json");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load_from(&path).is_empty());
        put_in(&path, draft("1", "Premier")).unwrap();
        put_in(&path, draft("2", "Second")).unwrap();
        // Même identifiant : remplacé et remonté en tête.
        put_in(&path, draft("1", "Premier, revu")).unwrap();
        let drafts = load_from(&path);
        assert_eq!(drafts.len(), 2);
        assert_eq!(drafts[0].text, "Premier, revu");
        assert_eq!(drafts[1].id, "2");
        remove_in(&path, "2").unwrap();
        remove_in(&path, "inconnu").unwrap();
        assert_eq!(load_from(&path), vec![draft("1", "Premier, revu")]);
        std::fs::write(&path, "pas du JSON").unwrap();
        assert!(load_from(&path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
