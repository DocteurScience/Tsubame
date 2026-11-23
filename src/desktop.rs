//! Texte des notifications du bureau, d'après les notifications Bluesky.

use gettextrs::{gettext, ngettext};

use crate::data::Post;

/// Longueur maximale du texte repris dans la notification, en caractères.
const MAX_BODY: usize = 140;

/// Titre et texte de la notification du bureau : la plus récente non lue
/// (« Aya Koné a répondu à votre post » et son texte), puis le nombre des
/// autres. Sans notification non lue dans `posts`, seulement le nombre.
pub fn message(posts: &[Post], count: u32) -> (String, String) {
    let total = || {
        ngettext(
            "Vous avez {} notification non lue.",
            "Vous avez {} notifications non lues.",
            count,
        )
        .replace("{}", &count.to_string())
    };
    let Some(newest) = posts.iter().find(|post| post.unread) else {
        return (gettext("Nouvelles notifications"), total());
    };
    let context = newest.context.clone().unwrap_or_default();
    // Une notification regroupée nomme déjà la personne.
    let title = if newest.name.is_empty() || context.starts_with(&newest.name) {
        context
    } else {
        format!("{} {}", newest.name, context)
    };
    let mut body: String = newest.text.trim().chars().take(MAX_BODY).collect();
    if newest.text.trim().chars().count() > MAX_BODY {
        body.push('…');
    }
    if count > 1 {
        let others = ngettext(
            "Et {} autre notification non lue.",
            "Et {} autres notifications non lues.",
            count - 1,
        )
        .replace("{}", &(count - 1).to_string());
        body = if body.is_empty() {
            others
        } else {
            format!("{body}\n{others}")
        };
    }
    (title, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(name: &str, context: &str, text: &str, unread: bool) -> Post {
        let mut post = Post::unavailable();
        post.name = name.into();
        post.context = Some(context.into());
        post.text = text.into();
        post.unread = unread;
        post
    }

    #[test]
    fn the_newest_unread_notification_is_shown() {
        let posts = vec![
            notification("Lucie Morel", "vous suit", "", false),
            notification(
                "Aya Koné",
                "a répondu à votre post",
                "Super nouvelle.",
                true,
            ),
        ];
        assert_eq!(
            message(&posts, 1),
            (
                "Aya Koné a répondu à votre post".into(),
                "Super nouvelle.".into()
            )
        );
        let (_, body) = message(&posts, 3);
        assert_eq!(body, "Super nouvelle.\nEt 2 autres notifications non lues.");
    }

    #[test]
    fn grouped_and_missing_notifications() {
        let grouped = vec![notification(
            "Pierre Delmas",
            "Pierre Delmas et 4 autres ont aimé votre post",
            "",
            true,
        )];
        assert_eq!(
            message(&grouped, 1).0,
            "Pierre Delmas et 4 autres ont aimé votre post"
        );
        assert_eq!(
            message(&[], 2),
            (
                "Nouvelles notifications".into(),
                "Vous avez 2 notifications non lues.".into()
            )
        );
        let long = vec![notification(
            "A",
            "vous a mentionné",
            &"x".repeat(200),
            true,
        )];
        assert_eq!(message(&long, 1).1.chars().count(), MAX_BODY + 1);
    }
}
