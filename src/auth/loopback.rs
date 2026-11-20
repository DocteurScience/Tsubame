//! Serveur HTTP minimal sur 127.0.0.1 qui reçoit la redirection OAuth.
//!
//! Après la connexion dans le navigateur, le serveur d'autorisation renvoie
//! vers `http://127.0.0.1:<port>/callback?code=…&state=…&iss=…`. On répond
//! par une courte page, puis on rend la chaîne de requête à l'appelant.

use std::io;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub const CALLBACK_PATH: &str = "/callback";
const MAX_REQUEST: usize = 16 * 1024;

pub struct LoopbackServer {
    listener: TcpListener,
}

impl LoopbackServer {
    /// Écoute sur un port libre de l'interface locale.
    pub async fn bind() -> io::Result<Self> {
        Ok(Self {
            listener: TcpListener::bind(("127.0.0.1", 0)).await?,
        })
    }

    pub fn redirect_uri(&self) -> io::Result<String> {
        let port = self.listener.local_addr()?.port();
        Ok(format!("http://127.0.0.1:{port}{CALLBACK_PATH}"))
    }

    /// Attend la redirection et renvoie sa chaîne de requête. Les autres
    /// requêtes (icône du site…) reçoivent une réponse 404.
    pub async fn wait_for_callback(&self) -> io::Result<String> {
        loop {
            let (mut stream, _) = self.listener.accept().await?;
            let mut buffer = Vec::new();
            let mut chunk = [0u8; 2048];
            while !buffer.windows(4).any(|w| w == b"\r\n\r\n") && buffer.len() < MAX_REQUEST {
                let read = stream.read(&mut chunk).await?;
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
            }
            let request = String::from_utf8_lossy(&buffer);
            match callback_query(&request) {
                Some(query) => {
                    stream.write_all(response(200, &page()).as_bytes()).await?;
                    let _ = stream.shutdown().await;
                    return Ok(query.to_owned());
                }
                None => {
                    stream.write_all(response(404, "").as_bytes()).await?;
                    let _ = stream.shutdown().await;
                }
            }
        }
    }
}

/// Chaîne de requête d'une requête `GET /callback?…`, sinon `None`.
fn callback_query(request: &str) -> Option<&str> {
    let line = request.lines().next()?;
    let mut parts = line.split(' ');
    if parts.next()? != "GET" {
        return None;
    }
    let target = parts.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    (path == CALLBACK_PATH).then_some(query)
}

fn page() -> String {
    // Page affichée dans le navigateur : pas de traduction possible avant
    // de connaître la langue du navigateur, on reprend celle de l'interface.
    let title = gettextrs::gettext("Connexion à Tsubame");
    let text = gettextrs::gettext("Vous pouvez fermer cette page et revenir à Tsubame.");
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title>\
         <meta name=\"viewport\" content=\"width=device-width\">\
         <style>body{{font-family:system-ui,sans-serif;margin:20vh auto;max-width:28em;\
         padding:0 1em;text-align:center}}@media(prefers-color-scheme:dark)\
         {{body{{background:#1e1e1e;color:#fff}}}}</style></head>\
         <body><h1>{title}</h1><p>{text}</p></body></html>"
    )
}

fn response(status: u16, body: &str) -> String {
    let reason = if status == 200 { "OK" } else { "Not Found" };
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_callback_request() {
        let request = "GET /callback?code=abc&state=xyz&iss=https%3A%2F%2Fbsky.social HTTP/1.1\r\n\
                       Host: 127.0.0.1:1234\r\n\r\n";
        assert_eq!(
            callback_query(request),
            Some("code=abc&state=xyz&iss=https%3A%2F%2Fbsky.social")
        );
    }

    #[test]
    fn ignores_other_requests() {
        assert_eq!(callback_query("GET /favicon.ico HTTP/1.1\r\n\r\n"), None);
        assert_eq!(
            callback_query("POST /callback?code=a HTTP/1.1\r\n\r\n"),
            None
        );
        assert_eq!(callback_query(""), None);
    }

    #[tokio::test]
    async fn receives_the_redirect_from_the_browser() {
        let server = LoopbackServer::bind().await.unwrap();
        let uri = server.redirect_uri().unwrap();
        assert!(uri.starts_with("http://127.0.0.1:") && uri.ends_with("/callback"));
        let port = uri
            .trim_start_matches("http://127.0.0.1:")
            .trim_end_matches("/callback")
            .to_owned();

        let browser = tokio::spawn(async move {
            let address = format!("127.0.0.1:{port}");
            // Le navigateur demande d'abord l'icône du site…
            let mut stream = tokio::net::TcpStream::connect(&address).await.unwrap();
            stream
                .write_all(b"GET /favicon.ico HTTP/1.1\r\n\r\n")
                .await
                .unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).await.unwrap();
            assert!(reply.starts_with("HTTP/1.1 404"));
            // … puis suit la redirection.
            let mut stream = tokio::net::TcpStream::connect(&address).await.unwrap();
            stream
                .write_all(b"GET /callback?code=c&state=s HTTP/1.1\r\n\r\n")
                .await
                .unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).await.unwrap();
            assert!(reply.starts_with("HTTP/1.1 200"));
            assert!(reply.contains("Tsubame"));
        });

        let query = server.wait_for_callback().await.unwrap();
        assert_eq!(query, "code=c&state=s");
        browser.await.unwrap();
    }
}
