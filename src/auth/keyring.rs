//! Sessions OAuth gardées dans le trousseau système, jamais dans un fichier.
//!
//! `oo7` passe par le service Secret Service (GNOME Trousseau) hors Flatpak,
//! et par le portail Secret dans Flatpak.
//!
//! Chaque session est un élément du trousseau. Ses attributs (non secrets)
//! identifient le compte : DID, identifiant, et l'URI de redirection avec
//! laquelle la session a été ouverte, qui fait partie du `client_id` et sert
//! à la restaurer. Le secret est la session elle-même (jetons, clé DPoP), en
//! JSON.

use std::collections::HashMap;
use std::fmt;

use atrium_api::types::string::Did;
use atrium_common::store::Store;
use atrium_oauth::store::session::{Session, SessionStore};
use tokio::sync::OnceCell;

const ATTR_APPLICATION: &str = "application";
const ATTR_KIND: &str = "kind";
const ATTR_DID: &str = "did";
const ATTR_HANDLE: &str = "handle";
const ATTR_REDIRECT_URI: &str = "redirect-uri";
const KIND_SESSION: &str = "oauth-session";

/// Début des messages d'erreur. `atrium-oauth` ne transmet parfois que le
/// texte de l'erreur : ces préfixes permettent d'y reconnaître une session
/// illisible.
const KEYRING_PREFIX: &str = "trousseau : ";
pub const UNREADABLE_PREFIX: &str = "session illisible : ";

#[derive(Debug)]
pub enum KeyringError {
    Keyring(oo7::Error),
    Json(serde_json::Error),
}

impl fmt::Display for KeyringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyringError::Keyring(e) => write!(f, "{KEYRING_PREFIX}{e}"),
            KeyringError::Json(e) => write!(f, "{UNREADABLE_PREFIX}{e}"),
        }
    }
}

impl std::error::Error for KeyringError {}

impl From<oo7::Error> for KeyringError {
    fn from(e: oo7::Error) -> Self {
        KeyringError::Keyring(e)
    }
}

impl From<serde_json::Error> for KeyringError {
    fn from(e: serde_json::Error) -> Self {
        KeyringError::Json(e)
    }
}

async fn keyring() -> Result<&'static oo7::Keyring, KeyringError> {
    static KEYRING: OnceCell<oo7::Keyring> = OnceCell::const_new();
    Ok(KEYRING.get_or_try_init(oo7::Keyring::new).await?)
}

/// Compte dont une session est gardée dans le trousseau.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAccount {
    pub did: String,
    pub handle: String,
    pub redirect_uri: String,
}

fn session_attributes(did: &str) -> HashMap<&'static str, String> {
    HashMap::from([
        (ATTR_APPLICATION, crate::APP_ID.to_owned()),
        (ATTR_KIND, KIND_SESSION.to_owned()),
        (ATTR_DID, did.to_owned()),
    ])
}

fn all_sessions_attributes() -> HashMap<&'static str, String> {
    HashMap::from([
        (ATTR_APPLICATION, crate::APP_ID.to_owned()),
        (ATTR_KIND, KIND_SESSION.to_owned()),
    ])
}

/// Comptes connectés, d'après le trousseau.
pub async fn stored_accounts() -> Result<Vec<StoredAccount>, KeyringError> {
    let items = keyring()
        .await?
        .search_items(&all_sessions_attributes())
        .await?;
    let mut accounts = Vec::new();
    for item in items {
        let attributes = item.attributes().await?;
        if let (Some(did), Some(handle), Some(redirect_uri)) = (
            attributes.get(ATTR_DID),
            attributes.get(ATTR_HANDLE),
            attributes.get(ATTR_REDIRECT_URI),
        ) {
            accounts.push(StoredAccount {
                did: did.clone(),
                handle: handle.clone(),
                redirect_uri: redirect_uri.clone(),
            });
        }
    }
    Ok(accounts)
}

/// Magasin de sessions pour `atrium-oauth`, pour un compte donné. Le
/// client OAuth y écrit la session à la connexion et à chaque
/// renouvellement des jetons.
#[derive(Clone)]
pub struct KeyringSessionStore {
    handle: String,
    redirect_uri: String,
}

impl KeyringSessionStore {
    pub fn new(handle: &str, redirect_uri: &str) -> Self {
        Self {
            handle: handle.to_owned(),
            redirect_uri: redirect_uri.to_owned(),
        }
    }
}

impl Store<Did, Session> for KeyringSessionStore {
    type Error = KeyringError;

    async fn get(&self, key: &Did) -> Result<Option<Session>, Self::Error> {
        let items = keyring()
            .await?
            .search_items(&session_attributes(key.as_str()))
            .await?;
        match items.first() {
            Some(item) => Ok(Some(serde_json::from_slice(
                item.secret().await?.as_bytes(),
            )?)),
            None => Ok(None),
        }
    }

    async fn set(&self, key: Did, value: Session) -> Result<(), Self::Error> {
        let mut attributes = session_attributes(key.as_str());
        attributes.insert(ATTR_HANDLE, self.handle.clone());
        attributes.insert(ATTR_REDIRECT_URI, self.redirect_uri.clone());
        let label =
            gettextrs::gettext("Session Bluesky de @{} (Tsubame)").replace("{}", &self.handle);
        keyring()
            .await?
            .create_item(&label, &attributes, serde_json::to_vec(&value)?, true)
            .await?;
        Ok(())
    }

    async fn del(&self, key: &Did) -> Result<(), Self::Error> {
        keyring()
            .await?
            .delete(&session_attributes(key.as_str()))
            .await?;
        Ok(())
    }

    async fn clear(&self) -> Result<(), Self::Error> {
        keyring().await?.delete(&all_sessions_attributes()).await?;
        Ok(())
    }
}

impl SessionStore for KeyringSessionStore {}
