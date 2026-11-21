//! État partagé des interactions d'un post : aimé, reposté, compteurs.
//!
//! Un même post peut être affiché à plusieurs endroits (une colonne et le
//! volet de discussion, deux colonnes). Chaque carte s'abonne à l'état du
//! post dans ce registre ; aimer ou reposter met à jour toutes les cartes à
//! la fois, tout de suite, puis l'état est confirmé ou annulé selon la
//! réponse de Bluesky.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

pub use crate::api::Interaction;
use crate::data::Post;

/// État d'une interaction (j'aime ou repost) sur un post.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Toggle {
    pub active: bool,
    /// Adresse de mon enregistrement, pour l'annuler.
    pub record: Option<String>,
    pub count: u32,
    /// Requête en cours : les clics sont ignorés jusqu'à la réponse.
    pub busy: bool,
}

/// Ce que demande un clic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Aimer ou reposter.
    Do,
    /// Annuler, avec l'adresse de l'enregistrement à supprimer.
    Undo(Option<String>),
}

type Watcher = Box<dyn Fn(Interaction, &Toggle)>;

pub struct Interactions {
    pub key: String,
    pub uri: Option<String>,
    pub cid: Option<String>,
    /// Le post lui-même, pour y répondre.
    post: RefCell<Post>,
    like: RefCell<Toggle>,
    repost: RefCell<Toggle>,
    watchers: RefCell<Vec<Watcher>>,
}

/// Clé d'un post dans le registre : son adresse, ou pour un faux post une
/// clé tirée de son auteur et de sa date.
pub fn key(post: &Post) -> String {
    post.uri
        .clone()
        .unwrap_or_else(|| format!("demo:{}:{}", post.handle, post.date))
}

fn toggles(post: &Post) -> (Toggle, Toggle) {
    (
        Toggle {
            active: post.like_uri.is_some(),
            record: post.like_uri.clone(),
            count: post.likes,
            busy: false,
        },
        Toggle {
            active: post.repost_uri.is_some(),
            record: post.repost_uri.clone(),
            count: post.reposts,
            busy: false,
        },
    )
}

thread_local! {
    static REGISTRY: RefCell<HashMap<String, Weak<Interactions>>> = RefCell::default();
}

/// État du post, partagé avec les autres cartes qui l'affichent. Des données
/// plus récentes (actualisation) remplacent l'ancien état, sauf pendant une
/// requête.
pub fn register(post: &Post) -> Rc<Interactions> {
    let key = key(post);
    if let Some(existing) = find(&key) {
        existing.post.replace(post.clone());
        let (like, repost) = toggles(post);
        existing.refresh(Interaction::Like, like);
        existing.refresh(Interaction::Repost, repost);
        return existing;
    }
    let (like, repost) = toggles(post);
    let entry = Rc::new(Interactions {
        key: key.clone(),
        uri: post.uri.clone(),
        cid: post.cid.clone(),
        post: RefCell::new(post.clone()),
        like: RefCell::new(like),
        repost: RefCell::new(repost),
        watchers: RefCell::default(),
    });
    REGISTRY.with_borrow_mut(|registry| {
        if registry.len() > 2000 {
            registry.retain(|_, entry| entry.strong_count() > 0);
        }
        registry.insert(key, Rc::downgrade(&entry));
    });
    entry
}

pub fn find(key: &str) -> Option<Rc<Interactions>> {
    REGISTRY.with_borrow(|registry| registry.get(key).and_then(Weak::upgrade))
}

impl Interactions {
    fn cell(&self, which: Interaction) -> &RefCell<Toggle> {
        match which {
            Interaction::Like => &self.like,
            Interaction::Repost => &self.repost,
        }
    }

    pub fn post(&self) -> Post {
        self.post.borrow().clone()
    }

    pub fn get(&self, which: Interaction) -> Toggle {
        self.cell(which).borrow().clone()
    }

    /// Appelle `f` à chaque changement d'état.
    pub fn connect(&self, f: impl Fn(Interaction, &Toggle) + 'static) {
        self.watchers.borrow_mut().push(Box::new(f));
    }

    fn notify(&self, which: Interaction) {
        let toggle = self.get(which);
        for watcher in self.watchers.borrow().iter() {
            watcher(which, &toggle);
        }
    }

    fn refresh(&self, which: Interaction, fresh: Toggle) {
        let changed = {
            let mut toggle = self.cell(which).borrow_mut();
            if toggle.busy || *toggle == fresh {
                false
            } else {
                *toggle = fresh;
                true
            }
        };
        if changed {
            self.notify(which);
        }
    }

    /// Clic : bascule tout de suite (compteur compris) et dit quoi demander
    /// à Bluesky. `None` si une requête est déjà en cours.
    pub fn begin(&self, which: Interaction) -> Option<Change> {
        let change = {
            let mut toggle = self.cell(which).borrow_mut();
            if toggle.busy {
                return None;
            }
            toggle.busy = true;
            let change = if toggle.active {
                toggle.count = toggle.count.saturating_sub(1);
                Change::Undo(toggle.record.take())
            } else {
                toggle.count += 1;
                Change::Do
            };
            toggle.active = !toggle.active;
            change
        };
        self.notify(which);
        Some(change)
    }

    /// Réponse de Bluesky : en cas de succès, l'adresse de l'enregistrement
    /// créé (`None` après une annulation) ; en cas d'échec, l'état revient à
    /// ce qu'il était.
    pub fn finish(&self, which: Interaction, change: &Change, result: Result<Option<String>, ()>) {
        {
            let mut toggle = self.cell(which).borrow_mut();
            toggle.busy = false;
            match (result, change) {
                (Ok(record), _) => toggle.record = record,
                (Err(()), Change::Do) => {
                    toggle.active = false;
                    toggle.count = toggle.count.saturating_sub(1);
                }
                (Err(()), Change::Undo(record)) => {
                    toggle.active = true;
                    toggle.count += 1;
                    toggle.record = record.clone();
                }
            }
        }
        self.notify(which);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn post(uri: &str, likes: u32, like_uri: Option<&str>) -> Post {
        let mut post = crate::data::posts(crate::data::Feed::Following).remove(0);
        post.uri = Some(uri.to_owned());
        post.cid = Some("cid".to_owned());
        post.likes = likes;
        post.like_uri = like_uri.map(str::to_owned);
        post
    }

    #[test]
    fn cards_of_the_same_post_share_their_state() {
        let a = register(&post("at://x/app.bsky.feed.post/1", 5, None));
        let b = register(&post("at://x/app.bsky.feed.post/1", 5, None));
        assert!(Rc::ptr_eq(&a, &b));
        let seen = Rc::new(Cell::new(0));
        let counter = seen.clone();
        b.connect(move |_, toggle| counter.set(toggle.count));
        assert_eq!(a.begin(Interaction::Like), Some(Change::Do));
        assert_eq!(seen.get(), 6);
        assert!(b.get(Interaction::Like).active);
    }

    #[test]
    fn like_is_confirmed_or_rolled_back() {
        let entry = register(&post("at://x/app.bsky.feed.post/2", 5, None));
        let change = entry.begin(Interaction::Like).unwrap();
        // Un deuxième clic pendant la requête est ignoré.
        assert_eq!(entry.begin(Interaction::Like), None);
        entry.finish(
            Interaction::Like,
            &change,
            Ok(Some("at://moi/app.bsky.feed.like/a".into())),
        );
        let like = entry.get(Interaction::Like);
        assert!(like.active && !like.busy);
        assert_eq!(like.count, 6);
        assert_eq!(
            like.record.as_deref(),
            Some("at://moi/app.bsky.feed.like/a")
        );

        // Annuler, mais Bluesky refuse : on revient à « aimé ».
        let change = entry.begin(Interaction::Like).unwrap();
        assert_eq!(
            change,
            Change::Undo(Some("at://moi/app.bsky.feed.like/a".into()))
        );
        assert_eq!(entry.get(Interaction::Like).count, 5);
        entry.finish(Interaction::Like, &change, Err(()));
        let like = entry.get(Interaction::Like);
        assert!(like.active);
        assert_eq!(like.count, 6);
        assert_eq!(
            like.record.as_deref(),
            Some("at://moi/app.bsky.feed.like/a")
        );
    }

    #[test]
    fn fresh_data_replaces_the_state_except_during_a_request() {
        let entry = register(&post("at://x/app.bsky.feed.post/3", 5, None));
        register(&post(
            "at://x/app.bsky.feed.post/3",
            9,
            Some("at://moi/l/1"),
        ));
        assert_eq!(entry.get(Interaction::Like).count, 9);
        assert!(entry.get(Interaction::Like).active);

        let change = entry.begin(Interaction::Like).unwrap();
        register(&post(
            "at://x/app.bsky.feed.post/3",
            12,
            Some("at://moi/l/1"),
        ));
        assert_eq!(entry.get(Interaction::Like).count, 8);
        entry.finish(Interaction::Like, &change, Ok(None));
        assert!(!entry.get(Interaction::Like).active);
    }
}
