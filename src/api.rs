//! Lecture des fils et des discussions sur l'API de Bluesky, et conversion
//! des réponses en posts affichables (`data::Post`).

use std::collections::HashMap;
use std::sync::Arc;

use atrium_api::app::bsky::actor::defs::ProfileViewBasic;
use atrium_api::app::bsky::embed::record_with_media::ViewMediaRefs;
use atrium_api::app::bsky::feed::defs::{
    FeedViewPost, FeedViewPostReasonRefs, PostView, PostViewEmbedRefs, ReplyRefParentRefs,
    ThreadViewPostRepliesItem,
};
use atrium_api::app::bsky::feed::get_post_thread::OutputThreadRefs;
use atrium_api::app::bsky::notification::list_notifications::Notification;
use atrium_api::types::Union;
use chrono::{DateTime, Datelike, FixedOffset, Local, Timelike, Utc};
use gettextrs::{gettext, ngettext};

use crate::auth::Session;
use crate::data::{Actor, List, Media, Post, Profile, Reply, StrongRef, Topic};

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
    /// Notifications (`listNotifications`).
    Notifications,
    /// Posts d'un compte (`getAuthorFeed`), par son DID ou identifiant.
    Author { actor: String, filter: AuthorFilter },
    /// Posts aimés par un compte (`getActorLikes`) : seulement le sien.
    Likes(String),
    /// Recherche de posts (`searchPosts`).
    Search(String),
    /// Mes signets (`getBookmarks`).
    Bookmarks,
    /// Fil d'une liste de curation (`getListFeed`).
    List(String),
}

/// Onglets d'un profil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorFilter {
    /// Posts et reposts, sans les réponses.
    Posts,
    /// Posts et réponses.
    Replies,
    /// Posts avec image ou vidéo.
    Media,
}

impl AuthorFilter {
    fn api_name(self) -> &'static str {
        match self {
            AuthorFilter::Posts => "posts_no_replies",
            AuthorFilter::Replies => "posts_with_replies",
            AuthorFilter::Media => "posts_with_media",
        }
    }
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

pub(crate) fn request_failed(error: impl std::fmt::Display) -> ApiError {
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
        Source::Notifications => return notifications(&api, cursor).await,
        Source::Bookmarks => {
            let output = api
                .api
                .app
                .bsky
                .bookmark
                .get_bookmarks(
                    atrium_api::app::bsky::bookmark::get_bookmarks::ParametersData {
                        cursor,
                        limit,
                    }
                    .into(),
                )
                .await
                .map_err(request_failed)?;
            let now = Utc::now();
            return Ok(Page {
                posts: output
                    .data
                    .bookmarks
                    .iter()
                    .filter_map(|bookmark| match &bookmark.item {
                        Union::Refs(atrium_api::app::bsky::bookmark::defs::BookmarkViewItemRefs::AppBskyFeedDefsPostView(post)) => {
                            Some(post_from_view(post, now))
                        }
                        _ => None,
                    })
                    .collect(),
                cursor: output.data.cursor,
            });
        }
        Source::List(uri) => {
            let output = api
                .api
                .app
                .bsky
                .feed
                .get_list_feed(
                    atrium_api::app::bsky::feed::get_list_feed::ParametersData {
                        cursor,
                        limit,
                        list: uri,
                    }
                    .into(),
                )
                .await
                .map_err(request_failed)?;
            let now = Utc::now();
            return Ok(Page {
                posts: output
                    .data
                    .feed
                    .iter()
                    .map(|item| post_from_feed(item, now))
                    .collect(),
                cursor: output.data.cursor,
            });
        }
        Source::Search(query) => {
            let output = api
                .api
                .app
                .bsky
                .feed
                .search_posts(
                    atrium_api::app::bsky::feed::search_posts::ParametersData {
                        author: None,
                        cursor,
                        domain: None,
                        lang: None,
                        limit,
                        mentions: None,
                        q: query,
                        since: None,
                        sort: None,
                        tag: None,
                        until: None,
                        url: None,
                    }
                    .into(),
                )
                .await
                .map_err(request_failed)?;
            let now = Utc::now();
            return Ok(Page {
                posts: output
                    .data
                    .posts
                    .iter()
                    .map(|post| post_from_view(post, now))
                    .collect(),
                cursor: output.data.cursor,
            });
        }
        Source::Author { actor, filter } => {
            let output = api
                .api
                .app
                .bsky
                .feed
                .get_author_feed(
                    atrium_api::app::bsky::feed::get_author_feed::ParametersData {
                        actor: actor.parse().map_err(|_| invalid_actor())?,
                        cursor,
                        filter: Some(filter.api_name().to_owned()),
                        include_pins: Some(filter == AuthorFilter::Posts),
                        limit,
                    }
                    .into(),
                )
                .await
                .map_err(request_failed)?;
            (output.data.feed, output.data.cursor)
        }
        Source::Likes(actor) => {
            let output = api
                .api
                .app
                .bsky
                .feed
                .get_actor_likes(
                    atrium_api::app::bsky::feed::get_actor_likes::ParametersData {
                        actor: actor.parse().map_err(|_| invalid_actor())?,
                        cursor,
                        limit,
                    }
                    .into(),
                )
                .await
                .map_err(request_failed)?;
            (output.data.feed, output.data.cursor)
        }
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
    name_or_handle(author.display_name.as_deref(), author.handle.as_str())
}

/// Nom affiché, ou l'identifiant quand il est vide.
pub(crate) fn name_or_handle(display_name: Option<&str>, handle: &str) -> String {
    display_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(handle)
        .to_owned()
}

/// Racine de la discussion d'une réponse : `reply.root` de son
/// enregistrement.
fn reply_root(post: &PostView) -> Option<StrongRef> {
    let record = serde_json::to_value(&post.record).ok()?;
    let root = record.get("reply")?.get("root")?;
    Some(StrongRef {
        uri: root.get("uri")?.as_str()?.to_owned(),
        cid: root.get("cid")?.as_str()?.to_owned(),
    })
}

/// Texte du post : le champ `text` de l'enregistrement `app.bsky.feed.post`.
fn record_text(post: &PostView) -> String {
    serde_json::to_value(&post.record)
        .ok()
        .and_then(|record| record.get("text")?.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Première image ou vidéo jointe, ou `None` sans média.
fn media(post: &PostView) -> Option<Media> {
    use atrium_api::app::bsky::embed::{defs::AspectRatio, images, video};
    let ratio = |ratio: &Option<AspectRatio>| {
        ratio.as_ref().and_then(|r| {
            Some((
                u32::try_from(r.width.get()).ok()?,
                u32::try_from(r.height.get()).ok()?,
            ))
        })
    };
    let images = |view: &images::View| {
        let first = view.images.first()?;
        Some(Media {
            alt: first.alt.clone(),
            thumb: Some(first.thumb.clone()),
            aspect_ratio: ratio(&first.aspect_ratio),
            more: view.images.len().saturating_sub(1) as u32,
        })
    };
    let video = |view: &video::View| {
        Some(Media {
            alt: view.alt.clone().unwrap_or_default(),
            thumb: view.thumbnail.clone(),
            aspect_ratio: ratio(&view.aspect_ratio),
            more: 0,
        })
    };
    match post.embed.as_ref()? {
        Union::Refs(PostViewEmbedRefs::AppBskyEmbedImagesView(view)) => images(view),
        Union::Refs(PostViewEmbedRefs::AppBskyEmbedVideoView(view)) => video(view),
        Union::Refs(PostViewEmbedRefs::AppBskyEmbedRecordWithMediaView(with_media)) => {
            match &with_media.media {
                Union::Refs(ViewMediaRefs::AppBskyEmbedImagesView(view)) => images(view),
                Union::Refs(ViewMediaRefs::AppBskyEmbedVideoView(view)) => video(view),
                _ => None,
            }
        }
        _ => None,
    }
}

pub(crate) fn parse_datetime(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).ok()
}

fn count(value: Option<i64>) -> u32 {
    value.unwrap_or(0).clamp(0, i64::from(u32::MAX)) as u32
}

pub fn post_from_view(post: &PostView, now: DateTime<Utc>) -> Post {
    let indexed = parse_datetime(post.indexed_at.as_str());
    let viewer = post.viewer.as_ref();
    Post {
        uri: Some(post.uri.clone()),
        cid: serde_json::to_value(&post.cid)
            .ok()
            .and_then(|cid| cid.as_str().map(str::to_owned)),
        like_uri: viewer.and_then(|v| v.like.clone()),
        repost_uri: viewer.and_then(|v| v.repost.clone()),
        bookmarked: viewer.and_then(|v| v.bookmarked).unwrap_or(false),
        reply_root: reply_root(post),
        context: None,
        name: display_name(&post.author),
        handle: post.author.handle.as_str().to_owned(),
        time: indexed.map(|t| relative_time(t, now)).unwrap_or_default(),
        date: indexed.map(full_date).unwrap_or_default(),
        text: record_text(post),
        avatar: post.author.avatar.clone(),
        media: media(post),
        replies: count(post.reply_count),
        reposts: count(post.repost_count),
        likes: count(post.like_count),
        thread: Vec::new(),
        unread: false,
        labels: labels(post),
    }
}

/// Étiquettes du post et de son auteur, sans celles qui sont annulées.
fn labels(post: &PostView) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for label in post
        .labels
        .iter()
        .flatten()
        .chain(post.author.labels.iter().flatten())
    {
        if label.neg == Some(true) {
            labels.retain(|value| *value != label.val);
        } else if !labels.contains(&label.val) {
            labels.push(label.val.clone());
        }
    }
    labels
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
        avatar: post.author.avatar.clone(),
        name: display_name(&post.author),
        handle: post.author.handle.as_str().to_owned(),
        time: indexed.map(|t| relative_time(t, now)).unwrap_or_default(),
        text: record_text(post),
    }
}

/// Posts concernés par des notifications, par lots de 25 (`getPosts`).
async fn posts_by_uri(
    api: &crate::auth::Api,
    uris: Vec<String>,
) -> Result<HashMap<String, PostView>, ApiError> {
    let mut posts = HashMap::new();
    for batch in uris.chunks(25) {
        let output = api
            .api
            .app
            .bsky
            .feed
            .get_posts(
                atrium_api::app::bsky::feed::get_posts::ParametersData {
                    uris: batch.to_vec(),
                }
                .into(),
            )
            .await
            .map_err(request_failed)?;
        for post in output.data.posts {
            posts.insert(post.uri.clone(), post);
        }
    }
    Ok(posts)
}

async fn notifications(api: &crate::auth::Api, cursor: Option<String>) -> Result<Page, ApiError> {
    let output = api
        .api
        .app
        .bsky
        .notification
        .list_notifications(
            atrium_api::app::bsky::notification::list_notifications::ParametersData {
                cursor,
                limit: PAGE_SIZE.try_into().ok(),
                priority: None,
                reasons: None,
                seen_at: None,
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    let mut uris: Vec<String> = output
        .data
        .notifications
        .iter()
        .filter_map(|n| subject_uri(n).map(str::to_owned))
        .collect();
    uris.sort();
    uris.dedup();
    let posts = posts_by_uri(api, uris).await?;
    let now = Utc::now();
    Ok(Page {
        posts: output
            .data
            .notifications
            .iter()
            .filter_map(|n| post_from_notification(n, &posts, now))
            .collect(),
        cursor: output.data.cursor,
    })
}

/// Marque les notifications comme vues (`updateSeen`).
pub async fn mark_notifications_seen(session: Arc<Session>) -> Result<(), ApiError> {
    let api = session.api().await?;
    let now = atrium_api::types::string::Datetime::now();
    api.api
        .app
        .bsky
        .notification
        .update_seen(
            atrium_api::app::bsky::notification::update_seen::InputData { seen_at: now }.into(),
        )
        .await
        .map_err(request_failed)
}

/// Post dont parle une notification : la réponse, la mention ou la
/// citation elle-même, ou le post aimé ou reposté.
fn subject_uri(notification: &Notification) -> Option<&str> {
    match notification.reason.as_str() {
        "reply" | "mention" | "quote" => Some(&notification.uri),
        "like" | "repost" | "like-via-repost" | "repost-via-repost" => {
            notification.reason_subject.as_deref()
        }
        _ => None,
    }
}

fn notification_context(reason: &str) -> Option<String> {
    Some(match reason {
        "like" => gettext("a aimé votre post"),
        "repost" => gettext("a reposté votre post"),
        "follow" => gettext("vous suit"),
        "mention" => gettext("vous a mentionné"),
        "reply" => gettext("a répondu à votre post"),
        "quote" => gettext("a cité votre post"),
        "like-via-repost" => gettext("a aimé votre repost"),
        "repost-via-repost" => gettext("a reposté votre repost"),
        "starterpack-joined" => gettext("a rejoint Bluesky avec votre starter pack"),
        // Raisons apparues après cette version : ignorées.
        _ => return None,
    })
}

/// Carte d'une notification : la personne à l'origine (nom, avatar), ce
/// qu'elle a fait, et le post concerné (texte, image, compteurs). Ouvrir la
/// carte ouvre la discussion de ce post.
pub fn post_from_notification(
    notification: &Notification,
    posts: &HashMap<String, PostView>,
    now: DateTime<Utc>,
) -> Option<Post> {
    let context = notification_context(&notification.reason)?;
    let subject = subject_uri(notification).and_then(|uri| posts.get(uri));
    let mut post = match subject {
        Some(view) => post_from_view(view, now),
        // Abonnement, ou post supprimé depuis : le texte de l'enregistrement
        // de la notification, s'il y en a un.
        None => Post {
            uri: None,
            cid: None,
            like_uri: None,
            repost_uri: None,
            bookmarked: false,
            reply_root: None,
            context: None,
            name: String::new(),
            handle: String::new(),
            time: String::new(),
            date: String::new(),
            text: serde_json::to_value(&notification.record)
                .ok()
                .and_then(|record| record.get("text")?.as_str().map(str::to_owned))
                .unwrap_or_default(),
            avatar: None,
            media: None,
            replies: 0,
            reposts: 0,
            likes: 0,
            thread: Vec::new(),
            unread: false,
            labels: Vec::new(),
        },
    };
    let author = &notification.author;
    let indexed = parse_datetime(notification.indexed_at.as_str());
    post.context = Some(context);
    post.name = name_or_handle(author.display_name.as_deref(), author.handle.as_str());
    post.handle = author.handle.as_str().to_owned();
    post.avatar = author.avatar.clone();
    post.time = indexed.map(|t| relative_time(t, now)).unwrap_or_default();
    if post.date.is_empty() {
        post.date = indexed.map(full_date).unwrap_or_default();
    }
    post.unread = !notification.is_read;
    Some(post)
}

pub fn actor_from_view(actor: &atrium_api::app::bsky::actor::defs::ProfileView) -> Actor {
    Actor {
        did: actor.did.as_str().to_owned(),
        handle: actor.handle.as_str().to_owned(),
        name: name_or_handle(actor.display_name.as_deref(), actor.handle.as_str()),
        bio: actor.description.clone().unwrap_or_default(),
        avatar: actor.avatar.clone(),
        following: actor.viewer.as_ref().and_then(|v| v.following.clone()),
    }
}

/// Personnes à suivre (`getSuggestions`).
pub async fn suggestions(session: Arc<Session>) -> Result<Vec<Actor>, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .actor
        .get_suggestions(
            atrium_api::app::bsky::actor::get_suggestions::ParametersData {
                cursor: None,
                limit: 10.try_into().ok(),
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(output.data.actors.iter().map(actor_from_view).collect())
}

/// Recherche de personnes (`searchActors`).
pub async fn search_actors(session: Arc<Session>, query: String) -> Result<Vec<Actor>, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .actor
        .search_actors(
            atrium_api::app::bsky::actor::search_actors::ParametersData {
                cursor: None,
                limit: 25.try_into().ok(),
                q: Some(query),
                term: None,
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(output.data.actors.iter().map(actor_from_view).collect())
}

/// Sujets du moment (`getTrendingTopics`).
pub async fn trending(session: Arc<Session>) -> Result<Vec<Topic>, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .unspecced
        .get_trending_topics(
            atrium_api::app::bsky::unspecced::get_trending_topics::ParametersData {
                limit: 10.try_into().ok(),
                viewer: None,
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(output
        .data
        .topics
        .iter()
        .map(|topic| Topic {
            name: topic
                .display_name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| topic.topic.clone()),
            query: topic.topic.clone(),
        })
        .collect())
}

/// Fil épinglé : son adresse et son nom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedFeed {
    pub uri: String,
    pub name: String,
}

/// Adresses des fils épinglés, dans l'ordre, d'après les préférences
/// (format `savedFeedsPrefV2`, ou l'ancien `savedFeedsPref`). Les listes et
/// le fil Following, qui ne sont pas des générateurs de fil, sont laissés
/// de côté.
pub fn pinned_feed_uris(preferences: &serde_json::Value) -> Vec<String> {
    let items = preferences.as_array().cloned().unwrap_or_default();
    let of_type = |name: &str| {
        items
            .iter()
            .find(|item| item.get("$type").and_then(|t| t.as_str()) == Some(name))
    };
    if let Some(v2) = of_type("app.bsky.actor.defs#savedFeedsPrefV2") {
        return v2
            .get("items")
            .and_then(|items| items.as_array())
            .into_iter()
            .flatten()
            .filter(|item| {
                item.get("pinned").and_then(|p| p.as_bool()) == Some(true)
                    && item.get("type").and_then(|t| t.as_str()) == Some("feed")
            })
            .filter_map(|item| item.get("value")?.as_str().map(str::to_owned))
            .collect();
    }
    of_type("app.bsky.actor.defs#savedFeedsPref")
        .and_then(|legacy| legacy.get("pinned")?.as_array().cloned())
        .into_iter()
        .flatten()
        .filter_map(|uri| uri.as_str().map(str::to_owned))
        .filter(|uri| uri.contains("/app.bsky.feed.generator/"))
        .collect()
}

/// Fils épinglés du compte, avec leurs noms.
pub async fn pinned_feeds(session: Arc<Session>) -> Result<Vec<PinnedFeed>, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .actor
        .get_preferences(atrium_api::app::bsky::actor::get_preferences::ParametersData {}.into())
        .await
        .map_err(request_failed)?;
    let preferences = serde_json::to_value(&output.data.preferences).unwrap_or_default();
    let uris = pinned_feed_uris(&preferences);
    if uris.is_empty() {
        return Ok(Vec::new());
    }
    let output = api
        .api
        .app
        .bsky
        .feed
        .get_feed_generators(
            atrium_api::app::bsky::feed::get_feed_generators::ParametersData {
                feeds: uris.clone(),
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    let names: HashMap<&str, &str> = output
        .data
        .feeds
        .iter()
        .map(|feed| (feed.uri.as_str(), feed.display_name.as_str()))
        .collect();
    // Dans l'ordre des préférences ; un fil disparu depuis est ignoré.
    Ok(uris
        .iter()
        .filter_map(|uri| {
            names.get(uri.as_str()).map(|name| PinnedFeed {
                uri: uri.clone(),
                name: (*name).to_owned(),
            })
        })
        .collect())
}

fn invalid_actor() -> ApiError {
    ApiError::new(gettext("Ce compte n'existe pas."))
}

pub fn profile_from_view(
    profile: &atrium_api::app::bsky::actor::defs::ProfileViewDetailed,
) -> Profile {
    let viewer = profile.viewer.as_ref();
    let count = |value: Option<i64>| value.unwrap_or(0).max(0) as u64;
    Profile {
        did: profile.did.as_str().to_owned(),
        handle: profile.handle.as_str().to_owned(),
        name: name_or_handle(profile.display_name.as_deref(), profile.handle.as_str()),
        bio: profile.description.clone().unwrap_or_default(),
        avatar: profile.avatar.clone(),
        banner: profile.banner.clone(),
        followers: count(profile.followers_count),
        follows: count(profile.follows_count),
        posts: count(profile.posts_count),
        following: viewer.and_then(|v| v.following.clone()),
        follows_me: viewer.is_some_and(|v| v.followed_by.is_some()),
        muted: viewer.and_then(|v| v.muted).unwrap_or(false),
        blocking: viewer.and_then(|v| v.blocking.clone()),
    }
}

/// Profil d'un compte, par son DID ou identifiant.
pub async fn profile(session: Arc<Session>, actor: String) -> Result<Profile, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .actor
        .get_profile(
            atrium_api::app::bsky::actor::get_profile::ParametersData {
                actor: actor.parse().map_err(|_| invalid_actor())?,
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(profile_from_view(&output))
}

/// Suit le compte `did` ; renvoie l'adresse de l'abonnement, qui sert à
/// l'annuler avec `delete`.
pub async fn follow(session: Arc<Session>, did: String) -> Result<String, ApiError> {
    use atrium_api::com::atproto::repo::create_record;
    use atrium_api::types::string::{Datetime, Did, Nsid};

    let api = session.api().await?;
    let record = serde_json::json!({
        "$type": "app.bsky.graph.follow",
        "subject": did,
        "createdAt": Datetime::now().as_str(),
    });
    let output = api
        .api
        .com
        .atproto
        .repo
        .create_record(
            create_record::InputData {
                collection: Nsid::new("app.bsky.graph.follow".to_owned())
                    .map_err(|_| invalid_actor())?,
                record: serde_json::from_value(record).map_err(|_| invalid_actor())?,
                repo: Did::new(session.account.did.clone())
                    .map_err(|_| invalid_actor())?
                    .into(),
                rkey: None,
                swap_commit: None,
                validate: None,
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(output.data.uri)
}

/// Ce qu'on peut faire sur un post : l'aimer ou le reposter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interaction {
    Like,
    Repost,
    /// Signet : privé, sans enregistrement dans le dépôt.
    Bookmark,
}

impl Interaction {
    fn collection(self) -> &'static str {
        match self {
            Interaction::Like => "app.bsky.feed.like",
            Interaction::Repost => "app.bsky.feed.repost",
            Interaction::Bookmark => "app.bsky.bookmark",
        }
    }
}

/// Ajoute un post aux signets.
pub async fn bookmark(session: Arc<Session>, uri: String, cid: String) -> Result<(), ApiError> {
    let api = session.api().await?;
    api.api
        .app
        .bsky
        .bookmark
        .create_bookmark(
            atrium_api::app::bsky::bookmark::create_bookmark::InputData {
                cid: cid.parse().map_err(|_| invalid_post())?,
                uri,
            }
            .into(),
        )
        .await
        .map_err(request_failed)
}

/// Retire un post des signets.
pub async fn unbookmark(session: Arc<Session>, uri: String) -> Result<(), ApiError> {
    let api = session.api().await?;
    api.api
        .app
        .bsky
        .bookmark
        .delete_bookmark(atrium_api::app::bsky::bookmark::delete_bookmark::InputData { uri }.into())
        .await
        .map_err(request_failed)
}

fn list_from_view(list: &atrium_api::app::bsky::graph::defs::ListView) -> List {
    let purpose = serde_json::to_value(&list.purpose).unwrap_or_default();
    List {
        uri: list.uri.clone(),
        name: list.name.clone(),
        description: list.description.clone().unwrap_or_default(),
        avatar: list.avatar.clone(),
        members: list.list_item_count.unwrap_or(0) as u64,
        moderation: purpose.as_str() == Some("app.bsky.graph.defs#modlist"),
    }
}

/// Listes créées par un compte.
pub async fn lists(session: Arc<Session>, actor: String) -> Result<Vec<List>, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .graph
        .get_lists(
            atrium_api::app::bsky::graph::get_lists::ParametersData {
                actor: actor.parse().map_err(|_| invalid_actor())?,
                cursor: None,
                limit: 100.try_into().ok(),
                purposes: None,
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(output.data.lists.iter().map(list_from_view).collect())
}

/// Membres d'une liste.
pub async fn list_members(session: Arc<Session>, uri: String) -> Result<Vec<Actor>, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .graph
        .get_list(
            atrium_api::app::bsky::graph::get_list::ParametersData {
                cursor: None,
                limit: 100.try_into().ok(),
                list: uri,
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(output
        .data
        .items
        .iter()
        .map(|item| actor_from_view(&item.subject))
        .collect())
}

fn invalid_post() -> ApiError {
    ApiError::new(gettext("Ce post ne peut pas être modifié."))
}

/// Découpe une adresse `at://dépôt/collection/clé`.
pub fn parse_record_uri(uri: &str) -> Option<(&str, &str, &str)> {
    let mut parts = uri.strip_prefix("at://")?.split('/');
    let (repo, collection, rkey) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || [repo, collection, rkey].iter().any(|p| p.is_empty()) {
        return None;
    }
    Some((repo, collection, rkey))
}

/// Aime ou reposte le post `uri` (de contenu `cid`) ; renvoie l'adresse de
/// l'enregistrement créé, qui sert à l'annuler.
pub async fn create(
    session: Arc<Session>,
    interaction: Interaction,
    uri: String,
    cid: String,
) -> Result<String, ApiError> {
    use atrium_api::com::atproto::repo::{create_record, strong_ref};
    use atrium_api::types::TryIntoUnknown;
    use atrium_api::types::string::{Datetime, Did, Nsid};

    // Un signet n'est pas un enregistrement : voir `bookmark`.
    if interaction == Interaction::Bookmark {
        return Err(invalid_post());
    }
    let api = session.api().await?;
    let subject: strong_ref::Main = strong_ref::MainData {
        cid: cid.parse().map_err(|_| invalid_post())?,
        uri,
    }
    .into();
    let record = match interaction {
        Interaction::Bookmark => unreachable!("traité plus haut"),
        Interaction::Like => atrium_api::app::bsky::feed::like::Record::from(
            atrium_api::app::bsky::feed::like::RecordData {
                created_at: Datetime::now(),
                subject,
                via: None,
            },
        )
        .try_into_unknown(),
        Interaction::Repost => atrium_api::app::bsky::feed::repost::Record::from(
            atrium_api::app::bsky::feed::repost::RecordData {
                created_at: Datetime::now(),
                subject,
                via: None,
            },
        )
        .try_into_unknown(),
    }
    .map_err(|_| invalid_post())?;
    let output = api
        .api
        .com
        .atproto
        .repo
        .create_record(
            create_record::InputData {
                collection: Nsid::new(interaction.collection().to_owned())
                    .map_err(|_| invalid_post())?,
                record,
                repo: Did::new(session.account.did.clone())
                    .map_err(|_| invalid_post())?
                    .into(),
                rkey: None,
                swap_commit: None,
                validate: None,
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(output.data.uri)
}

/// Annule un « j'aime » ou un repost : supprime son enregistrement.
pub async fn delete(session: Arc<Session>, record_uri: String) -> Result<(), ApiError> {
    use atrium_api::com::atproto::repo::delete_record;
    use atrium_api::types::string::{Nsid, RecordKey};

    let (repo, collection, rkey) = parse_record_uri(&record_uri).ok_or_else(invalid_post)?;
    let api = session.api().await?;
    api.api
        .com
        .atproto
        .repo
        .delete_record(
            delete_record::InputData {
                collection: Nsid::new(collection.to_owned()).map_err(|_| invalid_post())?,
                repo: repo.parse().map_err(|_| invalid_post())?,
                rkey: RecordKey::new(rkey.to_owned()).map_err(|_| invalid_post())?,
                swap_commit: None,
                swap_record: None,
            }
            .into(),
        )
        .await
        .map(|_| ())
        .map_err(request_failed)
}

/// Post à publier.
#[derive(Debug, Clone)]
pub struct NewPost {
    pub text: String,
    /// Langues du post (codes BCP 47 : « fr », « en »…).
    pub langs: Vec<String>,
    /// Réponse : la racine de la discussion et le post auquel on répond.
    pub reply: Option<(StrongRef, StrongRef)>,
    /// Images jointes (4 au plus), prêtes à l'envoi.
    pub images: Vec<NewImage>,
}

#[derive(Debug, Clone)]
pub struct NewImage {
    pub prepared: crate::photo::Prepared,
    pub alt: String,
}

/// Enregistrement `app.bsky.feed.post`. Les mentions dont l'identifiant
/// n'a pas été résolu (`dids`) restent du texte simple.
/// `blobs` : les images envoyées, dans l'ordre de `post.images`.
pub fn post_record(
    post: &NewPost,
    dids: &HashMap<String, String>,
    created_at: &str,
    blobs: &[serde_json::Value],
) -> serde_json::Value {
    use crate::richtext::{Feature, detect};
    use serde_json::json;

    let facets: Vec<_> = detect(&post.text)
        .into_iter()
        .filter_map(|facet| {
            let feature = match facet.feature {
                Feature::Link(uri) => json!({"$type": "app.bsky.richtext.facet#link", "uri": uri}),
                Feature::Tag(tag) => json!({"$type": "app.bsky.richtext.facet#tag", "tag": tag}),
                Feature::Mention(handle) => json!({
                    "$type": "app.bsky.richtext.facet#mention",
                    "did": dids.get(&handle)?,
                }),
            };
            Some(json!({
                "index": {"byteStart": facet.start, "byteEnd": facet.end},
                "features": [feature],
            }))
        })
        .collect();
    let mut record = json!({
        "$type": "app.bsky.feed.post",
        "text": post.text,
        "createdAt": created_at,
    });
    if !post.langs.is_empty() {
        record["langs"] = json!(post.langs);
    }
    if !facets.is_empty() {
        record["facets"] = json!(facets);
    }
    if !blobs.is_empty() {
        let images: Vec<_> = post
            .images
            .iter()
            .zip(blobs)
            .map(|(image, blob)| {
                json!({
                    "alt": image.alt,
                    "image": blob,
                    "aspectRatio": {
                        "width": image.prepared.width,
                        "height": image.prepared.height,
                    },
                })
            })
            .collect();
        record["embed"] = json!({"$type": "app.bsky.embed.images", "images": images});
    }
    if let Some((root, parent)) = &post.reply {
        record["reply"] = json!({
            "root": {"uri": root.uri, "cid": root.cid},
            "parent": {"uri": parent.uri, "cid": parent.cid},
        });
    }
    record
}

/// Publie un post ; renvoie sa référence.
pub async fn publish(session: Arc<Session>, post: NewPost) -> Result<StrongRef, ApiError> {
    use atrium_api::com::atproto::repo::create_record;
    use atrium_api::types::string::{Datetime, Did, Handle, Nsid};

    let api = session.api().await?;
    // Mentions : identifiant → DID. Un identifiant inconnu reste du texte.
    let mut dids = HashMap::new();
    for facet in crate::richtext::detect(&post.text) {
        if let crate::richtext::Feature::Mention(handle) = facet.feature
            && !dids.contains_key(&handle)
            && let Ok(valid) = Handle::new(handle.clone())
            && let Ok(output) = api
                .api
                .com
                .atproto
                .identity
                .resolve_handle(
                    atrium_api::com::atproto::identity::resolve_handle::ParametersData {
                        handle: valid,
                    }
                    .into(),
                )
                .await
        {
            dids.insert(handle, output.data.did.as_str().to_owned());
        }
    }
    // Images : envoyées une à une avant le post qui les cite.
    let mut blobs = Vec::new();
    for image in &post.images {
        let output = api
            .api
            .com
            .atproto
            .repo
            .upload_blob(image.prepared.bytes.clone())
            .await
            .map_err(|e| {
                ApiError::new(
                    gettext("Une image n'a pas pu être envoyée : {}").replace("{}", &e.to_string()),
                )
            })?;
        blobs.push(serde_json::to_value(&output.data.blob).map_err(|_| invalid_post())?);
    }
    let created_at = Datetime::now();
    let record = post_record(&post, &dids, created_at.as_str(), &blobs);
    let record = serde_json::from_value(record).map_err(|_| invalid_post())?;
    let output = api
        .api
        .com
        .atproto
        .repo
        .create_record(
            create_record::InputData {
                collection: Nsid::new("app.bsky.feed.post".to_owned())
                    .map_err(|_| invalid_post())?,
                record,
                repo: Did::new(session.account.did.clone())
                    .map_err(|_| invalid_post())?
                    .into(),
                rkey: None,
                swap_commit: None,
                validate: None,
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(StrongRef {
        uri: output.data.uri,
        cid: serde_json::to_value(&output.data.cid)
            .ok()
            .and_then(|cid| cid.as_str().map(str::to_owned))
            .unwrap_or_default(),
    })
}

pub(crate) fn month(month: u32) -> String {
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
                "displayName": "Lucie Morel",
                "avatar": "https://cdn.example/avatar/lucie.jpg"
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
                    "alt": "Capture du sélecteur de fichiers",
                    "aspectRatio": {"width": 1600, "height": 900}
                }, {
                    "thumb": "https://cdn.example/thumb2.jpg",
                    "fullsize": "https://cdn.example/full2.jpg",
                    "alt": ""
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
            post.avatar.as_deref(),
            Some("https://cdn.example/avatar/lucie.jpg")
        );
        assert_eq!(
            post.media,
            Some(Media {
                alt: "Capture du sélecteur de fichiers".into(),
                thumb: Some("https://cdn.example/thumb.jpg".into()),
                aspect_ratio: Some((1600, 900)),
                more: 1,
            })
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
    fn labels_of_the_post_and_its_author_are_kept() {
        let post: PostView = serde_json::from_str(
            r#"{
                "uri": "at://did:plc:r/app.bsky.feed.post/1",
                "cid": "bafyreie5737gdxlw5i64vzichcalba3z2v5n6icifvx5xytvske7mr3hpm",
                "author": {"did": "did:plc:r", "handle": "romain.example.social",
                    "labels": [{"src": "did:plc:m", "uri": "did:plc:r", "val": "nudity", "cts": "2026-11-20T14:56:00.000Z"}]},
                "record": {"$type": "app.bsky.feed.post", "text": "Bonjour", "createdAt": "2026-11-20T14:56:00.000Z"},
                "labels": [
                    {"src": "did:plc:m", "uri": "at://did:plc:r/app.bsky.feed.post/1", "val": "graphic-media", "cts": "2026-11-20T14:56:00.000Z"},
                    {"src": "did:plc:m", "uri": "at://did:plc:r/app.bsky.feed.post/1", "val": "porn", "cts": "2026-11-20T14:56:00.000Z"},
                    {"src": "did:plc:m", "uri": "at://did:plc:r/app.bsky.feed.post/1", "val": "porn", "neg": true, "cts": "2026-11-20T14:57:00.000Z"}
                ],
                "indexedAt": "2026-11-20T14:56:00.000Z"
            }"#,
        )
        .unwrap();
        assert_eq!(
            post_from_view(&post, now()).labels,
            vec!["graphic-media".to_owned(), "nudity".to_owned()]
        );
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
        assert_eq!(post.media, None);
        assert_eq!(post.avatar, None);
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

    fn notification(reason: &str, uri: &str, subject: Option<&str>, record: &str) -> Notification {
        let subject = subject
            .map(|s| format!(r#""reasonSubject": "{s}","#))
            .unwrap_or_default();
        serde_json::from_str(&format!(
            r#"{{
                "uri": "{uri}",
                "cid": "bafyreie5737gdxlw5i64vzichcalba3z2v5n6icifvx5xytvske7mr3hpm",
                "author": {{
                    "did": "did:plc:pierre",
                    "handle": "pierre.example.social",
                    "displayName": "Pierre Delmas",
                    "avatar": "https://cdn.example/avatar/pierre.jpg"
                }},
                "reason": "{reason}",
                {subject}
                "record": {record},
                "isRead": false,
                "indexedAt": "2026-11-20T14:20:00.000Z"
            }}"#
        ))
        .unwrap()
    }

    fn my_post() -> PostView {
        serde_json::from_str(
            r#"{
                "uri": "at://did:plc:moi/app.bsky.feed.post/1",
                "cid": "bafyreie5737gdxlw5i64vzichcalba3z2v5n6icifvx5xytvske7mr3hpm",
                "author": {"did": "did:plc:moi", "handle": "vous.bsky.social"},
                "record": {"$type": "app.bsky.feed.post", "text": "Ajoutez un texte alternatif.", "createdAt": "2026-11-20T10:00:00.000Z"},
                "likeCount": 20,
                "indexedAt": "2026-11-20T10:00:00.000Z"
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn likes_show_who_liked_and_the_liked_post() {
        let like = notification(
            "like",
            "at://did:plc:pierre/app.bsky.feed.like/9",
            Some("at://did:plc:moi/app.bsky.feed.post/1"),
            r#"{"$type": "app.bsky.feed.like", "subject": {"uri": "at://did:plc:moi/app.bsky.feed.post/1", "cid": "x"}, "createdAt": "2026-11-20T14:20:00.000Z"}"#,
        );
        let posts = HashMap::from([(my_post().uri.clone(), my_post())]);
        let post = post_from_notification(&like, &posts, now()).unwrap();
        assert_eq!(post.context.as_deref(), Some("a aimé votre post"));
        assert_eq!(post.name, "Pierre Delmas");
        assert_eq!(
            post.avatar.as_deref(),
            Some("https://cdn.example/avatar/pierre.jpg")
        );
        assert_eq!(post.text, "Ajoutez un texte alternatif.");
        assert_eq!(post.likes, 20);
        assert_eq!(
            post.uri.as_deref(),
            Some("at://did:plc:moi/app.bsky.feed.post/1")
        );
        assert_eq!(post.time, "40 min");
        assert!(post.unread);
    }

    #[test]
    fn replies_show_the_reply_itself() {
        let uri = "at://did:plc:pierre/app.bsky.feed.post/r";
        let reply = notification(
            "reply",
            uri,
            Some("at://did:plc:moi/app.bsky.feed.post/1"),
            r#"{"$type": "app.bsky.feed.post", "text": "Bien vu.", "createdAt": "2026-11-20T14:20:00.000Z"}"#,
        );
        let view: PostView = serde_json::from_str(&format!(
            r#"{{
                "uri": "{uri}",
                "cid": "bafyreie5737gdxlw5i64vzichcalba3z2v5n6icifvx5xytvske7mr3hpm",
                "author": {{"did": "did:plc:pierre", "handle": "pierre.example.social"}},
                "record": {{"$type": "app.bsky.feed.post", "text": "Bien vu.", "createdAt": "2026-11-20T14:20:00.000Z"}},
                "replyCount": 1,
                "indexedAt": "2026-11-20T14:20:00.000Z"
            }}"#
        ))
        .unwrap();
        let posts = HashMap::from([(uri.to_owned(), view)]);
        let post = post_from_notification(&reply, &posts, now()).unwrap();
        assert_eq!(post.context.as_deref(), Some("a répondu à votre post"));
        assert_eq!(post.text, "Bien vu.");
        assert_eq!(post.uri.as_deref(), Some(uri));
        assert_eq!(post.replies, 1);
    }

    #[test]
    fn follows_deleted_posts_and_unknown_reasons() {
        let follow = notification(
            "follow",
            "at://did:plc:pierre/app.bsky.graph.follow/1",
            None,
            r#"{"$type": "app.bsky.graph.follow", "subject": "did:plc:moi", "createdAt": "2026-11-20T14:20:00.000Z"}"#,
        );
        let post = post_from_notification(&follow, &HashMap::new(), now()).unwrap();
        assert_eq!(post.context.as_deref(), Some("vous suit"));
        assert_eq!((post.uri, post.text.as_str()), (None, ""));

        // Réponse supprimée depuis : le texte de la notification reste.
        let reply = notification(
            "reply",
            "at://did:plc:pierre/app.bsky.feed.post/gone",
            None,
            r#"{"$type": "app.bsky.feed.post", "text": "Message disparu", "createdAt": "2026-11-20T14:20:00.000Z"}"#,
        );
        let post = post_from_notification(&reply, &HashMap::new(), now()).unwrap();
        assert_eq!(post.text, "Message disparu");

        let unknown = notification(
            "nouveauté",
            "at://did:plc:pierre/app.bsky.feed.post/n",
            None,
            r#"{"$type": "app.bsky.feed.post", "text": "", "createdAt": "2026-11-20T14:20:00.000Z"}"#,
        );
        assert!(post_from_notification(&unknown, &HashMap::new(), now()).is_none());
    }

    #[test]
    fn record_uris_are_split() {
        assert_eq!(
            parse_record_uri("at://did:plc:moi/app.bsky.feed.like/3kxyz"),
            Some(("did:plc:moi", "app.bsky.feed.like", "3kxyz"))
        );
        assert_eq!(parse_record_uri("https://bsky.app/profile/x"), None);
        assert_eq!(
            parse_record_uri("at://did:plc:moi/app.bsky.feed.like"),
            None
        );
        assert_eq!(parse_record_uri("at://did:plc:moi//3kxyz"), None);
        assert_eq!(parse_record_uri("at://a/b/c/d"), None);
    }

    #[test]
    fn viewer_state_and_cid_are_kept() {
        let mut item: serde_json::Value = serde_json::from_str(FEED_ITEM).unwrap();
        item["post"]["viewer"] = serde_json::json!({
            "like": "at://did:plc:moi/app.bsky.feed.like/1",
            "repost": null
        });
        let item: FeedViewPost = serde_json::from_value(item).unwrap();
        let post = post_from_feed(&item, now());
        assert_eq!(
            post.cid.as_deref(),
            Some("bafyreie5737gdxlw5i64vzichcalba3z2v5n6icifvx5xytvske7mr3hpm")
        );
        assert_eq!(
            post.like_uri.as_deref(),
            Some("at://did:plc:moi/app.bsky.feed.like/1")
        );
        assert_eq!(post.repost_uri, None);
    }

    const CID: &str = "bafyreie5737gdxlw5i64vzichcalba3z2v5n6icifvx5xytvske7mr3hpm";

    #[test]
    fn new_posts_become_records() {
        let root = StrongRef {
            uri: "at://did:plc:lucie/app.bsky.feed.post/1".into(),
            cid: CID.into(),
        };
        let parent = StrongRef {
            uri: "at://did:plc:aya/app.bsky.feed.post/2".into(),
            cid: CID.into(),
        };
        let post = NewPost {
            text: "Merci @lucie.example.social et @inconnu.example ! #GNOME https://gnome.org"
                .into(),
            langs: vec!["fr".into()],
            reply: Some((root, parent)),
            images: Vec::new(),
        };
        let dids = HashMap::from([(
            "lucie.example.social".to_owned(),
            "did:plc:lucie".to_owned(),
        )]);
        let record = post_record(&post, &dids, "2026-11-20T15:00:00.000Z", &[]);
        assert_eq!(record["$type"], "app.bsky.feed.post");
        assert_eq!(record["langs"], serde_json::json!(["fr"]));
        assert_eq!(record["reply"]["root"]["cid"], CID);
        assert_eq!(
            record["reply"]["parent"]["uri"],
            "at://did:plc:aya/app.bsky.feed.post/2"
        );
        let facets = record["facets"].as_array().unwrap();
        // La mention inconnue reste du texte : trois facettes, pas quatre.
        assert_eq!(facets.len(), 3);
        assert_eq!(facets[0]["features"][0]["did"], "did:plc:lucie");
        assert_eq!(facets[0]["index"]["byteStart"], 6);
        assert_eq!(facets[1]["features"][0]["tag"], "GNOME");
        assert_eq!(facets[2]["features"][0]["uri"], "https://gnome.org");
        // L'enregistrement est accepté par les types du protocole.
        let _: atrium_api::app::bsky::feed::post::Record = serde_json::from_value(record).unwrap();
    }

    #[test]
    fn images_are_embedded_with_alt_text_and_proportions() {
        let image = |alt: &str| NewImage {
            prepared: crate::photo::Prepared {
                bytes: Vec::new(),
                mime: "image/jpeg",
                width: 2000,
                height: 1125,
            },
            alt: alt.into(),
        };
        let post = NewPost {
            text: String::new(),
            langs: vec!["fr".into()],
            reply: None,
            images: vec![image("Lever de soleil sur le Vercors"), image("")],
        };
        let blob = serde_json::json!({
            "$type": "blob",
            "ref": {"$link": CID},
            "mimeType": "image/jpeg",
            "size": 123456
        });
        let record = post_record(
            &post,
            &HashMap::new(),
            "2026-11-20T15:00:00.000Z",
            &[blob.clone(), blob],
        );
        let images = &record["embed"]["images"];
        assert_eq!(record["embed"]["$type"], "app.bsky.embed.images");
        assert_eq!(images[0]["alt"], "Lever de soleil sur le Vercors");
        assert_eq!(images[1]["alt"], "");
        assert_eq!(images[0]["aspectRatio"]["height"], 1125);
        assert_eq!(images[0]["image"]["size"], 123456);
        // Un post fait seulement d'images est valide.
        let _: atrium_api::app::bsky::feed::post::Record = serde_json::from_value(record).unwrap();
    }

    #[test]
    fn plain_posts_have_no_facets_nor_reply() {
        let post = NewPost {
            text: "Bonjour".into(),
            langs: Vec::new(),
            reply: None,
            images: Vec::new(),
        };
        let record = post_record(&post, &HashMap::new(), "2026-11-20T15:00:00.000Z", &[]);
        assert!(record.get("facets").is_none());
        assert!(record.get("reply").is_none());
        assert!(record.get("langs").is_none());
    }

    #[test]
    fn replies_keep_the_root_of_the_thread() {
        let mut item: serde_json::Value = serde_json::from_str(FEED_ITEM).unwrap();
        item["post"]["record"]["reply"] = serde_json::json!({
            "root": {"uri": "at://did:plc:x/app.bsky.feed.post/root", "cid": "cidroot"},
            "parent": {"uri": "at://did:plc:aya/app.bsky.feed.post/2", "cid": "cidparent"}
        });
        let item: FeedViewPost = serde_json::from_value(item).unwrap();
        let post = post_from_feed(&item, now());
        assert_eq!(
            post.reply_root,
            Some(StrongRef {
                uri: "at://did:plc:x/app.bsky.feed.post/root".into(),
                cid: "cidroot".into(),
            })
        );
    }

    #[test]
    fn profiles_are_read() {
        let view: atrium_api::app::bsky::actor::defs::ProfileViewDetailed = serde_json::from_str(
            r#"{
                    "did": "did:plc:lucie",
                    "handle": "lucie.example.social",
                    "displayName": "Lucie Morel",
                    "description": "Développeuse GTK et traductrice.",
                    "avatar": "https://cdn.example/avatar/lucie.jpg",
                    "followersCount": 1284,
                    "followsCount": 312,
                    "postsCount": 2906,
                    "viewer": {
                        "following": "at://did:plc:moi/app.bsky.graph.follow/1",
                        "followedBy": "at://did:plc:lucie/app.bsky.graph.follow/2",
                        "muted": true,
                        "blocking": "at://did:plc:moi/app.bsky.graph.block/3"
                    }
                }"#,
        )
        .unwrap();
        let profile = profile_from_view(&view);
        assert_eq!(profile.name, "Lucie Morel");
        assert_eq!(profile.bio, "Développeuse GTK et traductrice.");
        assert_eq!(
            (profile.followers, profile.follows, profile.posts),
            (1284, 312, 2906)
        );
        assert_eq!(
            profile.following.as_deref(),
            Some("at://did:plc:moi/app.bsky.graph.follow/1")
        );
        assert!(profile.follows_me);
        assert!(profile.muted);
        assert_eq!(
            profile.blocking.as_deref(),
            Some("at://did:plc:moi/app.bsky.graph.block/3")
        );
        assert_eq!(profile.banner, None);
    }

    #[test]
    fn people_are_read() {
        let view: atrium_api::app::bsky::actor::defs::ProfileView = serde_json::from_str(
            r#"{
                "did": "did:plc:nadia",
                "handle": "nadia.example.social",
                "description": "Traductrice",
                "viewer": {"following": "at://did:plc:moi/app.bsky.graph.follow/3"}
            }"#,
        )
        .unwrap();
        let actor = actor_from_view(&view);
        assert_eq!(actor.name, "nadia.example.social");
        assert_eq!(actor.bio, "Traductrice");
        assert_eq!(
            actor.following.as_deref(),
            Some("at://did:plc:moi/app.bsky.graph.follow/3")
        );
    }

    #[test]
    fn pinned_feeds_are_read_from_preferences() {
        let preferences = serde_json::json!([
            {"$type": "app.bsky.actor.defs#adultContentPref", "enabled": false},
            {"$type": "app.bsky.actor.defs#savedFeedsPrefV2", "items": [
                {"id": "1", "type": "timeline", "value": "following", "pinned": true},
                {"id": "2", "type": "feed", "value": "at://did:plc:a/app.bsky.feed.generator/linux", "pinned": true},
                {"id": "3", "type": "list", "value": "at://did:plc:a/app.bsky.graph.list/l", "pinned": true},
                {"id": "4", "type": "feed", "value": "at://did:plc:a/app.bsky.feed.generator/saved", "pinned": false},
                {"id": "5", "type": "feed", "value": "at://did:plc:b/app.bsky.feed.generator/photo", "pinned": true}
            ]}
        ]);
        assert_eq!(
            pinned_feed_uris(&preferences),
            vec![
                "at://did:plc:a/app.bsky.feed.generator/linux",
                "at://did:plc:b/app.bsky.feed.generator/photo",
            ]
        );
        // Ancien format.
        let legacy = serde_json::json!([
            {"$type": "app.bsky.actor.defs#savedFeedsPref",
             "pinned": ["at://did:plc:a/app.bsky.feed.generator/linux", "at://did:plc:a/app.bsky.graph.list/l"],
             "saved": []}
        ]);
        assert_eq!(
            pinned_feed_uris(&legacy),
            vec!["at://did:plc:a/app.bsky.feed.generator/linux"]
        );
        assert!(pinned_feed_uris(&serde_json::json!([])).is_empty());
    }

    #[test]
    fn bookmark_state_and_lists_are_read() {
        let mut item: serde_json::Value = serde_json::from_str(FEED_ITEM).unwrap();
        item["post"]["viewer"] = serde_json::json!({"bookmarked": true});
        let item: FeedViewPost = serde_json::from_value(item).unwrap();
        assert!(post_from_feed(&item, now()).bookmarked);

        let list: atrium_api::app::bsky::graph::defs::ListView =
            serde_json::from_value(serde_json::json!({
                "uri": "at://did:plc:moi/app.bsky.graph.list/1",
                "cid": CID,
                "creator": {"did": "did:plc:moi", "handle": "vous.bsky.social"},
                "name": "Comptes indésirables",
                "purpose": "app.bsky.graph.defs#modlist",
                "listItemCount": 3,
                "indexedAt": "2026-11-20T10:00:00.000Z"
            }))
            .unwrap();
        let list = list_from_view(&list);
        assert!(list.moderation);
        assert_eq!(list.members, 3);
        assert_eq!(list.description, "");
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
