//! Lecture des fils et des discussions sur l'API de Bluesky, et conversion
//! des réponses en posts affichables (`data::Post`).

use std::sync::Arc;

use atrium_api::app::bsky::actor::defs::ProfileViewBasic;
use atrium_api::app::bsky::embed::record_with_media::ViewMediaRefs;
use atrium_api::app::bsky::feed::defs::{
    FeedViewPost, FeedViewPostReasonRefs, PostView, PostViewEmbedRefs, ReplyRefParentRefs,
    ThreadViewPostRepliesItem,
};
use atrium_api::app::bsky::feed::get_post_thread::OutputThreadRefs;
use atrium_api::types::Union;
use chrono::{DateTime, Datelike, FixedOffset, Local, Timelike, Utc};
use gettextrs::{gettext, ngettext};

use crate::auth::Session;
use crate::data::{Post, Reply};

/// Fil « Discover » de Bluesky.
pub const DISCOVER_FEED: &str =
    "at://did:plc:z72i7hdynmk6r22z27h6tvur/app.bsky.feed.generator/whats-hot";

/// Nombre de posts demandés par page.
const PAGE_SIZE: u8 = 30;

/// D'où viennent les posts d'une colonne.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Fil Following (`getTimeline`).
    Timeline,
    /// Fil personnalisé, par son adresse `at://` (`getFeed`).
    Feed(String),
}

/// Une page de posts et le curseur de la suivante.
pub struct Page {
    pub posts: Vec<Post>,
    pub cursor: Option<String>,
}

/// Échec d'une requête, avec un message pour l'utilisateur.
#[derive(Debug, Clone)]
pub struct ApiError {
    pub message: String,
    /// La session n'est plus utilisable : il faut se reconnecter.
    pub expired: bool,
}

impl ApiError {
    pub fn new(message: String) -> Self {
        Self {
            message,
            expired: false,
        }
    }
}

impl From<crate::auth::LoginError> for ApiError {
    fn from(e: crate::auth::LoginError) -> Self {
        Self {
            message: e.message(),
            expired: matches!(e, crate::auth::LoginError::Expired),
        }
    }
}

fn request_failed(error: impl std::fmt::Display) -> ApiError {
    ApiError::new(gettext("Bluesky n'a pas répondu : {}").replace("{}", &error.to_string()))
}

pub async fn load(
    session: Arc<Session>,
    source: Source,
    cursor: Option<String>,
) -> Result<Page, ApiError> {
    let api = session.api().await?;
    let limit = PAGE_SIZE.try_into().ok();
    let (feed, cursor) = match source {
        Source::Timeline => {
            let output = api
                .api
                .app
                .bsky
                .feed
                .get_timeline(
                    atrium_api::app::bsky::feed::get_timeline::ParametersData {
                        algorithm: None,
                        cursor,
                        limit,
                    }
                    .into(),
                )
                .await
                .map_err(request_failed)?;
            (output.data.feed, output.data.cursor)
        }
        Source::Feed(uri) => {
            let output = api
                .api
                .app
                .bsky
                .feed
                .get_feed(
                    atrium_api::app::bsky::feed::get_feed::ParametersData {
                        cursor,
                        feed: uri,
                        limit,
                    }
                    .into(),
                )
                .await
                .map_err(request_failed)?;
            (output.data.feed, output.data.cursor)
        }
    };
    let now = Utc::now();
    Ok(Page {
        posts: feed.iter().map(|item| post_from_feed(item, now)).collect(),
        cursor,
    })
}

/// Réponses directes à un post.
pub async fn replies(session: Arc<Session>, uri: String) -> Result<Vec<Reply>, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .feed
        .get_post_thread(
            atrium_api::app::bsky::feed::get_post_thread::ParametersData {
                depth: 1.try_into().ok(),
                parent_height: 0.try_into().ok(),
                uri,
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(replies_from_thread(&output.data.thread, Utc::now()))
}

fn replies_from_thread(thread: &Union<OutputThreadRefs>, now: DateTime<Utc>) -> Vec<Reply> {
    let Union::Refs(OutputThreadRefs::AppBskyFeedDefsThreadViewPost(thread)) = thread else {
        return Vec::new();
    };
    thread
        .replies
        .iter()
        .flatten()
        .filter_map(|reply| match reply {
            Union::Refs(ThreadViewPostRepliesItem::ThreadViewPost(reply)) => {
                Some(reply_from_view(&reply.post, now))
            }
            _ => None,
        })
        .collect()
}

fn display_name(author: &ProfileViewBasic) -> String {
    author
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| author.handle.as_str().to_owned())
}

/// Texte du post : le champ `text` de l'enregistrement `app.bsky.feed.post`.
fn record_text(post: &PostView) -> String {
    serde_json::to_value(&post.record)
        .ok()
        .and_then(|record| record.get("text")?.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Texte alternatif de la première image ou vidéo jointe (vide s'il
/// manque), ou `None` sans média.
fn media_alt(post: &PostView) -> Option<String> {
    let first_image = |images: &atrium_api::app::bsky::embed::images::View| {
        images.images.first().map(|image| image.alt.clone())
    };
    let video = |video: &atrium_api::app::bsky::embed::video::View| {
        Some(video.alt.clone().unwrap_or_default())
    };
    match post.embed.as_ref()? {
        Union::Refs(PostViewEmbedRefs::AppBskyEmbedImagesView(images)) => first_image(images),
        Union::Refs(PostViewEmbedRefs::AppBskyEmbedVideoView(v)) => video(v),
        Union::Refs(PostViewEmbedRefs::AppBskyEmbedRecordWithMediaView(with_media)) => {
            match &with_media.media {
                Union::Refs(ViewMediaRefs::AppBskyEmbedImagesView(images)) => first_image(images),
                Union::Refs(ViewMediaRefs::AppBskyEmbedVideoView(v)) => video(v),
                _ => None,
            }
        }
        _ => None,
    }
}

fn parse_datetime(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).ok()
}

fn count(value: Option<i64>) -> u32 {
    value.unwrap_or(0).clamp(0, i64::from(u32::MAX)) as u32
}

pub fn post_from_view(post: &PostView, now: DateTime<Utc>) -> Post {
    let indexed = parse_datetime(post.indexed_at.as_str());
    Post {
        uri: Some(post.uri.clone()),
        context: None,
        name: display_name(&post.author),
        handle: post.author.handle.as_str().to_owned(),
        time: indexed.map(|t| relative_time(t, now)).unwrap_or_default(),
        date: indexed.map(full_date).unwrap_or_default(),
        text: record_text(post),
        image_alt: media_alt(post),
        replies: count(post.reply_count),
        reposts: count(post.repost_count),
        likes: count(post.like_count),
        thread: Vec::new(),
    }
}

pub fn post_from_feed(item: &FeedViewPost, now: DateTime<Utc>) -> Post {
    let mut post = post_from_view(&item.post, now);
    let reposted_by = match &item.reason {
        Some(Union::Refs(FeedViewPostReasonRefs::ReasonRepost(reason))) => {
            Some(display_name(&reason.by))
        }
        _ => None,
    };
    let replying_to = item.reply.as_ref().and_then(|reply| match &reply.parent {
        Union::Refs(ReplyRefParentRefs::PostView(parent)) => Some(display_name(&parent.author)),
        _ => None,
    });
    post.context = match (reposted_by, replying_to) {
        (Some(by), _) => Some(gettext("{} a reposté").replace("{}", &by)),
        (None, Some(to)) => Some(gettext("En réponse à {}").replace("{}", &to)),
        (None, None) => None,
    };
    post
}

fn reply_from_view(post: &PostView, now: DateTime<Utc>) -> Reply {
    let indexed = parse_datetime(post.indexed_at.as_str());
    Reply {
        name: display_name(&post.author),
        handle: post.author.handle.as_str().to_owned(),
        time: indexed.map(|t| relative_time(t, now)).unwrap_or_default(),
        text: record_text(post),
    }
}

fn month(month: u32) -> String {
    match month {
        1 => gettext("janv."),
        2 => gettext("févr."),
        3 => gettext("mars"),
        4 => gettext("avr."),
        5 => gettext("mai"),
        6 => gettext("juin"),
        7 => gettext("juil."),
        8 => gettext("août"),
        9 => gettext("sept."),
        10 => gettext("oct."),
        11 => gettext("nov."),
        _ => gettext("déc."),
    }
}

/// Heure relative des cartes : « à l'instant », « 4 min », « 2 h », « 3 j »,
/// puis la date au-delà d'une semaine.
pub fn relative_time(then: DateTime<FixedOffset>, now: DateTime<Utc>) -> String {
    let elapsed = now.signed_duration_since(then);
    let minutes = elapsed.num_minutes();
    if minutes < 1 {
        gettext("à l'instant")
    } else if minutes < 60 {
        gettext("{} min").replace("{}", &minutes.to_string())
    } else if elapsed.num_hours() < 24 {
        gettext("{} h").replace("{}", &elapsed.num_hours().to_string())
    } else if elapsed.num_days() < 7 {
        let days = elapsed.num_days();
        ngettext("{} j", "{} j", days as u32).replace("{}", &days.to_string())
    } else {
        let local = then.with_timezone(&Local);
        let current_year = now.with_timezone(&Local).year();
        if local.year() == current_year {
            format!("{} {}", local.day(), month(local.month()))
        } else {
            format!("{} {} {}", local.day(), month(local.month()), local.year())
        }
    }
}

/// Date complète du volet de discussion : « 20 nov. 2026, 14:32 ».
pub fn full_date(then: DateTime<FixedOffset>) -> String {
    let local = then.with_timezone(&Local);
    format!(
        "{} {} {}, {:02}:{:02}",
        local.day(),
        month(local.month()),
        local.year(),
        local.hour(),
        local.minute()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-20T15:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn at(value: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(value).unwrap()
    }

    /// Un post du fil Following tel que l'API le renvoie : repost d'une
    /// réponse, avec une image.
    const FEED_ITEM: &str = r#"{
        "post": {
            "uri": "at://did:plc:lucie/app.bsky.feed.post/3kabc",
            "cid": "bafyreie5737gdxlw5i64vzichcalba3z2v5n6icifvx5xytvske7mr3hpm",
            "author": {
                "did": "did:plc:lucie",
                "handle": "lucie.example.social",
                "displayName": "Lucie Morel"
            },
            "record": {
                "$type": "app.bsky.feed.post",
                "text": "Notes de version de GNOME 49.",
                "createdAt": "2026-11-20T14:31:00.000Z"
            },
            "embed": {
                "$type": "app.bsky.embed.images#view",
                "images": [{
                    "thumb": "https://cdn.example/thumb.jpg",
                    "fullsize": "https://cdn.example/full.jpg",
                    "alt": "Capture du sélecteur de fichiers"
                }]
            },
            "replyCount": 8,
            "repostCount": 12,
            "likeCount": 48,
            "indexedAt": "2026-11-20T14:32:00.000Z"
        },
        "reply": {
            "root": {
                "$type": "app.bsky.feed.defs#notFoundPost",
                "uri": "at://did:plc:x/app.bsky.feed.post/1",
                "notFound": true
            },
            "parent": {
                "$type": "app.bsky.feed.defs#postView",
                "uri": "at://did:plc:aya/app.bsky.feed.post/2",
                "cid": "bafyreie5737gdxlw5i64vzichcalba3z2v5n6icifvx5xytvske7mr3hpm",
                "author": {"did": "did:plc:aya", "handle": "aya.example.social", "displayName": "Aya Koné"},
                "record": {"$type": "app.bsky.feed.post", "text": "?", "createdAt": "2026-11-20T14:00:00.000Z"},
                "indexedAt": "2026-11-20T14:00:00.000Z"
            }
        },
        "reason": {
            "$type": "app.bsky.feed.defs#reasonRepost",
            "by": {"did": "did:plc:nadia", "handle": "nadia.example.social", "displayName": ""},
            "indexedAt": "2026-11-20T14:50:00.000Z"
        }
    }"#;

    #[test]
    fn feed_items_become_cards() {
        let item: FeedViewPost = serde_json::from_str(FEED_ITEM).unwrap();
        let post = post_from_feed(&item, now());
        assert_eq!(
            post.uri.as_deref(),
            Some("at://did:plc:lucie/app.bsky.feed.post/3kabc")
        );
        assert_eq!(post.name, "Lucie Morel");
        assert_eq!(post.handle, "lucie.example.social");
        assert_eq!(post.text, "Notes de version de GNOME 49.");
        assert_eq!(post.time, "28 min");
        assert_eq!(
            post.image_alt.as_deref(),
            Some("Capture du sélecteur de fichiers")
        );
        assert_eq!((post.replies, post.reposts, post.likes), (8, 12, 48));
        // Repost par un compte sans nom affiché : l'identifiant le remplace.
        assert_eq!(
            post.context.as_deref(),
            Some("nadia.example.social a reposté")
        );
    }

    #[test]
    fn replies_mention_their_parent() {
        let mut item: serde_json::Value = serde_json::from_str(FEED_ITEM).unwrap();
        item.as_object_mut().unwrap().remove("reason");
        let item: FeedViewPost = serde_json::from_value(item).unwrap();
        let post = post_from_feed(&item, now());
        assert_eq!(post.context.as_deref(), Some("En réponse à Aya Koné"));
    }

    #[test]
    fn posts_without_media_or_counts() {
        let post: PostView = serde_json::from_str(
            r#"{
                "uri": "at://did:plc:r/app.bsky.feed.post/1",
                "cid": "bafyreie5737gdxlw5i64vzichcalba3z2v5n6icifvx5xytvske7mr3hpm",
                "author": {"did": "did:plc:r", "handle": "romain.example.social"},
                "record": {"$type": "app.bsky.feed.post", "text": "Bonjour", "createdAt": "2026-11-20T14:56:00.000Z"},
                "indexedAt": "2026-11-20T14:56:00.000Z"
            }"#,
        )
        .unwrap();
        let post = post_from_view(&post, now());
        assert_eq!(post.name, "romain.example.social");
        assert_eq!(post.image_alt, None);
        assert_eq!((post.replies, post.reposts, post.likes), (0, 0, 0));
        assert_eq!(post.context, None);
    }

    #[test]
    fn thread_replies_are_listed() {
        let thread: Union<OutputThreadRefs> = serde_json::from_str(
            r#"{
                "$type": "app.bsky.feed.defs#threadViewPost",
                "post": {
                    "uri": "at://did:plc:lucie/app.bsky.feed.post/3kabc",
                    "cid": "bafyreie5737gdxlw5i64vzichcalba3z2v5n6icifvx5xytvske7mr3hpm",
                    "author": {"did": "did:plc:lucie", "handle": "lucie.example.social"},
                    "record": {"$type": "app.bsky.feed.post", "text": "GNOME 49", "createdAt": "2026-11-20T14:31:00.000Z"},
                    "indexedAt": "2026-11-20T14:32:00.000Z"
                },
                "replies": [
                    {
                        "$type": "app.bsky.feed.defs#threadViewPost",
                        "post": {
                            "uri": "at://did:plc:aya/app.bsky.feed.post/r1",
                            "cid": "bafyreie5737gdxlw5i64vzichcalba3z2v5n6icifvx5xytvske7mr3hpm",
                            "author": {"did": "did:plc:aya", "handle": "aya.example.social", "displayName": "Aya Koné"},
                            "record": {"$type": "app.bsky.feed.post", "text": "Super nouvelle.", "createdAt": "2026-11-20T14:40:00.000Z"},
                            "indexedAt": "2026-11-20T14:40:00.000Z"
                        }
                    },
                    {
                        "$type": "app.bsky.feed.defs#blockedPost",
                        "uri": "at://did:plc:b/app.bsky.feed.post/r2",
                        "blocked": true,
                        "author": {"did": "did:plc:b"}
                    }
                ]
            }"#,
        )
        .unwrap();
        let replies = replies_from_thread(&thread, now());
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].name, "Aya Koné");
        assert_eq!(replies[0].text, "Super nouvelle.");
        assert_eq!(replies[0].time, "20 min");
    }

    #[test]
    fn relative_times() {
        assert_eq!(
            relative_time(at("2026-11-20T14:59:40Z"), now()),
            "à l'instant"
        );
        assert_eq!(relative_time(at("2026-11-20T14:56:00Z"), now()), "4 min");
        assert_eq!(relative_time(at("2026-11-20T13:00:00Z"), now()), "2 h");
        assert_eq!(relative_time(at("2026-11-17T15:00:00Z"), now()), "3 j");
        // Au-delà d'une semaine, la date (le jour dépend du fuseau local).
        let older = relative_time(at("2026-10-02T12:00:00Z"), now());
        assert!(older.ends_with("oct."), "{older}");
        let last_year = relative_time(at("2025-06-15T12:00:00Z"), now());
        assert!(last_year.ends_with("juin 2025"), "{last_year}");
    }
}
