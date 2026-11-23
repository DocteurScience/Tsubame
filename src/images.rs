//! Téléchargement des images (avatars, miniatures) et cache en mémoire.
//!
//! Les téléchargements tournent sur le runtime tokio, au plus
//! `MAX_DOWNLOADS` à la fois ; le décodage aussi, hors de la boucle GTK. Une
//! image demandée plusieurs fois (avatar présent sur plusieurs cartes) n'est
//! téléchargée qu'une fois.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;

use gtk::{gdk, glib};
use tokio::sync::Semaphore;

use crate::runtime::runtime;

/// Images gardées en mémoire ; au-delà, les plus anciennes sont oubliées.
const CAPACITY: usize = 400;
const MAX_DOWNLOADS: usize = 6;
/// Taille maximale d'une image téléchargée.
const MAX_BYTES: usize = 10 * 1024 * 1024;

type Waiting = Vec<Box<dyn FnOnce(&gdk::Texture)>>;

#[derive(Default)]
struct Cache {
    textures: HashMap<String, gdk::Texture>,
    order: VecDeque<String>,
    pending: HashMap<String, Waiting>,
}

impl Cache {
    fn insert(&mut self, url: String, texture: gdk::Texture) {
        if self.textures.insert(url.clone(), texture).is_none() {
            self.order.push_back(url);
        }
        while self.order.len() > CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.textures.remove(&oldest);
            }
        }
    }
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::default();
}

/// Charge l'image de `url` et appelle `f` avec elle, tout de suite si elle
/// est en mémoire. En cas d'échec, `f` n'est pas appelée : l'emplacement
/// (initiales de l'avatar, fond de l'image) reste affiché.
pub fn load(url: &str, f: impl FnOnce(&gdk::Texture) + 'static) {
    let cached = CACHE.with_borrow(|cache| cache.textures.get(url).cloned());
    if let Some(texture) = cached {
        f(&texture);
        return;
    }
    let already_loading = CACHE.with_borrow_mut(|cache| match cache.pending.get_mut(url) {
        Some(waiting) => {
            waiting.push(Box::new(f));
            true
        }
        None => {
            cache.pending.insert(url.to_owned(), vec![Box::new(f)]);
            false
        }
    });
    if already_loading {
        return;
    }

    let task = runtime().spawn(texture(url.to_owned()));
    let url = url.to_owned();
    glib::spawn_future_local(async move {
        let result = task.await.map_err(|e| e.to_string()).and_then(|r| r);
        let waiting = CACHE.with_borrow_mut(|cache| cache.pending.remove(&url));
        match result {
            Ok(texture) => {
                CACHE.with_borrow_mut(|cache| cache.insert(url, texture.clone()));
                for f in waiting.into_iter().flatten() {
                    f(&texture);
                }
            }
            Err(error) => glib::g_debug!("tsubame", "image {url} : {error}"),
        }
    });
}

pub(crate) fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("Tsubame/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("client HTTP")
    })
}

/// Télécharge les octets d'une image, en refusant ce qui dépasse
/// `MAX_BYTES`.
pub(crate) async fn download(url: &str) -> Result<Vec<u8>, String> {
    static SLOTS: Semaphore = Semaphore::const_new(MAX_DOWNLOADS);
    let _slot = SLOTS.acquire().await.map_err(|e| e.to_string())?;
    let mut response = client()
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BYTES as u64)
    {
        return Err("image trop lourde".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err("image trop lourde".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn texture(url: String) -> Result<gdk::Texture, String> {
    let bytes = download(&url).await?;
    tokio::task::spawn_blocking(move || {
        gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::prelude::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Sert une seule réponse HTTP sur 127.0.0.1 et renvoie l'adresse.
    async fn serve_once(body: Vec<u8>) -> String {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).await;
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
        });
        format!("http://{address}/avatar.png")
    }

    /// Plus petite image PNG valide : 1 × 1 pixel.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
        0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    #[tokio::test]
    async fn downloads_and_decodes_an_image() {
        let url = serve_once(PNG.to_vec()).await;
        let texture = texture(url).await.unwrap();
        assert_eq!((texture.width(), texture.height()), (1, 1));
    }

    #[tokio::test]
    async fn rejects_what_is_not_an_image() {
        let url = serve_once(b"<html>pas une image</html>".to_vec()).await;
        assert!(texture(url).await.is_err());
    }

    #[test]
    fn cache_forgets_the_oldest_images() {
        let texture = gdk::MemoryTexture::new(
            1,
            1,
            gdk::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_static(&[0, 0, 0, 255]),
            4,
        );
        let mut cache = Cache::default();
        for i in 0..CAPACITY + 10 {
            cache.insert(format!("https://cdn.example/{i}"), texture.clone().into());
        }
        assert_eq!(cache.textures.len(), CAPACITY);
        assert!(!cache.textures.contains_key("https://cdn.example/0"));
        assert!(
            cache
                .textures
                .contains_key(&format!("https://cdn.example/{}", CAPACITY + 9))
        );
    }
}
