//! Connexion à Bluesky avec OAuth (`atrium-oauth`).
//!
//! Déroulé :
//! 1. `start` ouvre un serveur local (`loopback`), résout l'identifiant et
//!    prépare l'adresse d'autorisation ;
//! 2. l'interface ouvre cette adresse dans le navigateur ;
//! 3. `PendingLogin::finish` attend la redirection, échange le code contre
//!    des jetons et garde la session dans le trousseau (`keyring`).
//!
//! Client OAuth : pour l'instant le client de développement « localhost »
//! du protocole AT, qui ne demande aucun fichier publié. Une version publiée
//! devra utiliser un `client_id` en https (métadonnées du client hébergées
//! en ligne) ; voir `docs/design.md`.

pub mod keyring;
pub mod loopback;

use std::sync::Arc;

use atrium_api::agent::Agent;
use atrium_api::agent::SessionManager;
use atrium_api::types::string::{Did, Handle};
use atrium_common::store::Store;
use atrium_identity::did::{CommonDidResolver, CommonDidResolverConfig, DEFAULT_PLC_DIRECTORY_URL};
use atrium_identity::handle::{AtprotoHandleResolver, AtprotoHandleResolverConfig, DnsTxtResolver};
use atrium_oauth::store::state::MemoryStateStore;
use atrium_oauth::{
    AtprotoLocalhostClientMetadata, AuthorizeOptions, CallbackParams, DefaultHttpClient,
    KnownScope, OAuthClient, OAuthClientConfig, OAuthResolverConfig, OAuthSession, Scope,
};
use gettextrs::gettext;

use keyring::{KeyringError, KeyringSessionStore};
use loopback::LoopbackServer;

/// Compte connecté.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub did: String,
    pub handle: String,
}

/// Échec de connexion, avec un message pour l'utilisateur.
#[derive(Debug)]
pub enum LoginError {
    InvalidHandle,
    AccountNotFound,
    /// Plus de session pour ce compte dans le trousseau.
    Expired,
    /// Identité introuvable faute de réponse : compte inexistant ou réseau
    /// absent, on ne peut pas trancher.
    Unreachable,
    Denied,
    Keyring(String),
    Failed(String),
}

impl LoginError {
    pub fn message(&self) -> String {
        match self {
            LoginError::InvalidHandle => {
                gettext("Cet identifiant n'est pas valide. Exemple : vous.bsky.social")
            }
            LoginError::AccountNotFound => gettext("Aucun compte trouvé pour cet identifiant."),
            LoginError::Expired => gettext("Votre session a expiré. Reconnectez-vous."),
            LoginError::Unreachable => gettext(
                "Impossible de trouver ce compte. Vérifiez l'identifiant et votre connexion à Internet.",
            ),
            LoginError::Denied => gettext("La connexion a été refusée dans le navigateur."),
            LoginError::Keyring(detail) => {
                gettext("Impossible d'enregistrer la session dans le trousseau : {}")
                    .replace("{}", detail)
            }
            LoginError::Failed(detail) => {
                gettext("La connexion a échoué : {}").replace("{}", detail)
            }
        }
    }
}

impl From<KeyringError> for LoginError {
    fn from(e: KeyringError) -> Self {
        LoginError::Keyring(e.to_string())
    }
}

impl From<atrium_oauth::Error> for LoginError {
    fn from(e: atrium_oauth::Error) -> Self {
        match e {
            atrium_oauth::Error::Identity(atrium_identity::Error::NotFound)
            | atrium_oauth::Error::Identity(atrium_identity::Error::AtIdentifier(_)) => {
                LoginError::AccountNotFound
            }
            atrium_oauth::Error::Identity(
                atrium_identity::Error::HttpClient(_)
                | atrium_identity::Error::DnsResolver(_)
                | atrium_identity::Error::HttpStatus(_),
            ) => LoginError::Unreachable,
            e @ (atrium_oauth::Error::SessionRegistry(_) | atrium_oauth::Error::ServerAgent(_))
                if session_unusable(&e.to_string()) =>
            {
                LoginError::Expired
            }
            atrium_oauth::Error::SessionStore(e) => LoginError::Keyring(e.to_string()),
            e => LoginError::Failed(e.to_string()),
        }
    }
}

/// La session est-elle inutilisable (absente, illisible, jetons qui ne se
/// renouvellent plus) ? `atrium-oauth` ne rend pas publics les types de ces
/// erreurs : on les reconnaît à leur texte, celui de sa version 0.2.
fn session_unusable(message: &str) -> bool {
    [
        "session does not exist",
        "no refresh token available",
        "token response verification failed",
        keyring::UNREADABLE_PREFIX,
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

/// Met en forme ce que l'utilisateur a saisi : sans espaces ni « @ », en
/// minuscules. Accepte un identifiant (`vous.bsky.social`) ou un DID.
pub fn normalize_identifier(input: &str) -> Result<String, LoginError> {
    let input = input.trim().trim_start_matches('@');
    if input.starts_with("did:") {
        return Did::new(input.to_owned())
            .map(|did| did.as_str().to_owned())
            .map_err(|_| LoginError::InvalidHandle);
    }
    let handle = input.to_lowercase();
    Handle::new(handle)
        .map(|handle| handle.as_str().to_owned())
        .map_err(|_| LoginError::InvalidHandle)
}

pub struct HickoryDnsTxtResolver {
    resolver: hickory_resolver::TokioResolver,
}

impl DnsTxtResolver for HickoryDnsTxtResolver {
    async fn resolve(
        &self,
        query: &str,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync + 'static>> {
        let lookup = self.resolver.txt_lookup(query).await?;
        Ok(lookup
            .answers()
            .iter()
            .filter_map(|record| match &record.data {
                hickory_resolver::proto::rr::RData::TXT(txt) => Some(txt.to_string()),
                _ => None,
            })
            .collect())
    }
}

type Client = OAuthClient<
    MemoryStateStore,
    KeyringSessionStore,
    CommonDidResolver<DefaultHttpClient>,
    AtprotoHandleResolver<HickoryDnsTxtResolver, DefaultHttpClient>,
    DefaultHttpClient,
>;

fn scopes() -> Vec<Scope> {
    vec![
        Scope::Known(KnownScope::Atproto),
        Scope::Known(KnownScope::TransitionGeneric),
        Scope::Known(KnownScope::TransitionChatBsky),
    ]
}

/// Client OAuth pour une URI de redirection donnée. Avec le client
/// « localhost », cette URI fait partie du `client_id` : une session doit
/// être restaurée avec celle qui l'a ouverte.
fn client(handle: &str, redirect_uri: &str) -> Result<Client, LoginError> {
    let http_client = Arc::new(DefaultHttpClient::default());
    let dns = hickory_resolver::TokioResolver::builder_tokio()
        .and_then(|builder| builder.build())
        .map_err(|e| LoginError::Failed(e.to_string()))?;
    let config = OAuthClientConfig {
        client_metadata: AtprotoLocalhostClientMetadata {
            redirect_uris: Some(vec![redirect_uri.to_owned()]),
            scopes: Some(scopes()),
        },
        keys: None,
        resolver: OAuthResolverConfig {
            did_resolver: CommonDidResolver::new(CommonDidResolverConfig {
                plc_directory_url: DEFAULT_PLC_DIRECTORY_URL.to_owned(),
                http_client: Arc::clone(&http_client),
            }),
            handle_resolver: AtprotoHandleResolver::new(AtprotoHandleResolverConfig {
                dns_txt_resolver: HickoryDnsTxtResolver { resolver: dns },
                http_client: Arc::clone(&http_client),
            }),
            authorization_server_metadata: Default::default(),
            protected_resource_metadata: Default::default(),
        },
        state_store: MemoryStateStore::default(),
        session_store: KeyringSessionStore::new(handle, redirect_uri),
    };
    OAuthClient::new(config).map_err(LoginError::from)
}

/// Connexion en attente de la réponse du navigateur.
pub struct PendingLogin {
    /// Adresse à ouvrir dans le navigateur.
    pub url: String,
    handle: String,
    server: LoopbackServer,
    client: Client,
}

/// Prépare la connexion de `identifier` (identifiant ou DID).
pub async fn start(identifier: String) -> Result<PendingLogin, LoginError> {
    let handle = normalize_identifier(&identifier)?;
    let server = LoopbackServer::bind()
        .await
        .map_err(|e| LoginError::Failed(e.to_string()))?;
    let redirect_uri = server
        .redirect_uri()
        .map_err(|e| LoginError::Failed(e.to_string()))?;
    let client = client(&handle, &redirect_uri)?;
    let url = client
        .authorize(
            &handle,
            AuthorizeOptions {
                scopes: scopes(),
                ..Default::default()
            },
        )
        .await?;
    Ok(PendingLogin {
        url,
        handle,
        server,
        client,
    })
}

impl PendingLogin {
    /// Attend la redirection du navigateur et termine la connexion. La
    /// session est alors dans le trousseau.
    pub async fn finish(self) -> Result<Arc<Session>, LoginError> {
        let query = self
            .server
            .wait_for_callback()
            .await
            .map_err(|e| LoginError::Failed(e.to_string()))?;
        let params = parse_callback(&query)?;
        let (session, _) = self.client.callback(params).await?;
        let did = session
            .did()
            .await
            .ok_or_else(|| LoginError::Failed(gettext("identité du compte inconnue")))?;
        let account = Account {
            did: did.as_str().to_owned(),
            handle: self.handle,
        };
        Ok(Session::with_api(account, Arc::new(Agent::new(session))))
    }
}

/// Paramètres de la redirection, ou le refus de l'utilisateur.
fn parse_callback(query: &str) -> Result<CallbackParams, LoginError> {
    #[derive(serde::Deserialize)]
    struct Refusal {
        error: Option<String>,
        error_description: Option<String>,
    }
    if let Ok(Refusal {
        error: Some(error),
        error_description,
    }) = serde_html_form::from_str::<Refusal>(query)
    {
        return Err(if error == "access_denied" {
            LoginError::Denied
        } else {
            LoginError::Failed(error_description.unwrap_or(error))
        });
    }
    serde_html_form::from_str(query).map_err(|e| LoginError::Failed(e.to_string()))
}

/// Compte dont la session est dans le trousseau, s'il y en a un.
pub async fn stored_session() -> Result<Option<Arc<Session>>, LoginError> {
    let accounts = keyring::stored_accounts().await?;
    Ok(accounts.into_iter().next().map(|account| {
        Session::new(Account {
            did: account.did,
            handle: account.handle,
        })
    }))
}

/// Accès à l'API de Bluesky avec la session OAuth d'un compte.
pub type Api = Agent<
    OAuthSession<
        DefaultHttpClient,
        CommonDidResolver<DefaultHttpClient>,
        AtprotoHandleResolver<HickoryDnsTxtResolver, DefaultHttpClient>,
        KeyringSessionStore,
    >,
>;

/// Compte connecté et accès à l'API.
///
/// Au lancement, la session est lue dans le trousseau mais pas encore
/// restaurée : la restauration interroge le serveur d'autorisation. Elle se
/// fait à la première requête, et recommence à la suivante si elle échoue
/// (hors ligne, par exemple).
pub struct Session {
    pub account: Account,
    api: tokio::sync::OnceCell<Arc<Api>>,
}

impl Session {
    fn new(account: Account) -> Arc<Self> {
        Arc::new(Self {
            account,
            api: tokio::sync::OnceCell::new(),
        })
    }

    fn with_api(account: Account, api: Arc<Api>) -> Arc<Self> {
        Arc::new(Self {
            account,
            api: tokio::sync::OnceCell::new_with(Some(api)),
        })
    }

    pub async fn api(&self) -> Result<Arc<Api>, LoginError> {
        self.api
            .get_or_try_init(|| async {
                let stored = keyring::stored_accounts()
                    .await?
                    .into_iter()
                    .find(|stored| stored.did == self.account.did)
                    .ok_or(LoginError::Expired)?;
                let did = Did::new(stored.did.clone()).map_err(|_| LoginError::Expired)?;
                let client = client(&stored.handle, &stored.redirect_uri)?;
                let session = client.restore(&did).await?;
                Ok(Arc::new(Agent::new(session)))
            })
            .await
            .cloned()
    }
}

/// Déconnecte le compte : révoque les jetons si le serveur répond, et
/// retire la session du trousseau dans tous les cas.
pub async fn logout(account: &Account) -> Result<(), LoginError> {
    let stored = keyring::stored_accounts()
        .await?
        .into_iter()
        .find(|stored| stored.did == account.did);
    let did = Did::new(account.did.clone()).map_err(|e| LoginError::Failed(e.to_string()))?;
    if let Some(stored) = stored
        && let Ok(client) = client(&stored.handle, &stored.redirect_uri)
    {
        let _ = client.revoke(&did).await;
    }
    KeyringSessionStore::new(&account.handle, "")
        .del(&did)
        .await
        .map_err(LoginError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_normalized() {
        assert_eq!(
            normalize_identifier("  @Vous.Bsky.Social ").unwrap(),
            "vous.bsky.social"
        );
        assert_eq!(
            normalize_identifier("did:plc:z72i7hdynmk6r22z27h6tvur").unwrap(),
            "did:plc:z72i7hdynmk6r22z27h6tvur"
        );
        assert!(matches!(
            normalize_identifier("pas un identifiant"),
            Err(LoginError::InvalidHandle)
        ));
        assert!(matches!(
            normalize_identifier(""),
            Err(LoginError::InvalidHandle)
        ));
    }

    #[test]
    fn callback_parameters_are_read() {
        let params = parse_callback("code=abc&state=xyz&iss=https%3A%2F%2Fbsky.social").unwrap();
        assert_eq!(params.code, "abc");
        assert_eq!(params.state.as_deref(), Some("xyz"));
        assert_eq!(params.iss.as_deref(), Some("https://bsky.social"));
    }

    #[test]
    fn refusal_in_the_browser_is_reported() {
        assert!(matches!(
            parse_callback("error=access_denied&state=xyz"),
            Err(LoginError::Denied)
        ));
        assert!(matches!(
            parse_callback("error=server_error&error_description=Oups"),
            Err(LoginError::Failed(detail)) if detail == "Oups"
        ));
    }

    #[test]
    fn unreachable_identity_gets_a_readable_message() {
        let network = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refusé");
        let error = LoginError::from(atrium_oauth::Error::Identity(
            atrium_identity::Error::HttpClient(Box::new(network)),
        ));
        assert!(matches!(error, LoginError::Unreachable));
        assert!(!error.message().contains("http"));
        assert!(matches!(
            LoginError::from(atrium_oauth::Error::Identity(
                atrium_identity::Error::NotFound
            )),
            LoginError::AccountNotFound
        ));
    }

    #[test]
    fn unusable_sessions_ask_to_log_in_again() {
        assert!(session_unusable("session does not exist"));
        assert!(session_unusable(&format!(
            "session store error: {}missing field `dpop_key`",
            keyring::UNREADABLE_PREFIX
        )));
        assert!(session_unusable("no refresh token available"));
        // Trousseau indisponible ou réseau absent : pas une session expirée.
        assert!(!session_unusable(
            "session store error: trousseau : pas de service"
        ));
        assert!(!session_unusable(
            "http client error: error sending request"
        ));
    }

    #[test]
    fn localhost_client_is_accepted() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            assert!(client("vous.bsky.social", "http://127.0.0.1:4567/callback").is_ok());
        });
    }
}
