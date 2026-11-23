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
use gettextrs::{gettext, ngettext, pgettext};

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
    /// Mentions, réponses et citations seulement.
    Mentions,
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
    /// Posts avec vidéo.
    Videos,
}

impl AuthorFilter {
    fn api_name(self) -> &'static str {
        match self {
            AuthorFilter::Posts => "posts_no_replies",
            AuthorFilter::Replies => "posts_with_replies",
            AuthorFilter::Media => "posts_with_media",
            AuthorFilter::Videos => "posts_with_video",
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
        Source::Notifications => return notifications(&api, cursor, None).await,
        Source::Mentions => {
            return notifications(&api, cursor, Some(MENTION_REASONS)).await;
        }
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
            video: None,
            images: view
                .images
                .iter()
                .map(|image| crate::data::Image {
                    thumb: image.thumb.clone(),
                    fullsize: image.fullsize.clone(),
                    alt: image.alt.clone(),
                })
                .collect(),
        })
    };
    let video = |view: &video::View| {
        Some(Media {
            alt: view.alt.clone().unwrap_or_default(),
            thumb: view.thumbnail.clone(),
            aspect_ratio: ratio(&view.aspect_ratio),
            more: 0,
            video: Some(view.playlist.clone()),
            images: Vec::new(),
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
        facets: serde_json::to_value(&post.record)
            .map(|record| crate::richtext::from_record(&record))
            .unwrap_or_default(),
        avatar: post.author.avatar.clone(),
        media: media(post),
        replies: count(post.reply_count),
        reposts: count(post.repost_count),
        likes: count(post.like_count),
        thread: Vec::new(),
        unread: false,
        labels: labels(post),
        pinned: viewer.and_then(|v| v.pinned).unwrap_or(false),
        quote: quote(post, now),
        link: link(post),
    }
}

/// Lien joint à `post`, seul ou avec une citation.
fn link(post: &PostView) -> Option<crate::data::Link> {
    use atrium_api::app::bsky::embed::external;
    let view: &external::View = match post.embed.as_ref()? {
        Union::Refs(PostViewEmbedRefs::AppBskyEmbedExternalView(view)) => view,
        Union::Refs(PostViewEmbedRefs::AppBskyEmbedRecordWithMediaView(with_media)) => {
            match &with_media.media {
                Union::Refs(ViewMediaRefs::AppBskyEmbedExternalView(view)) => view,
                _ => return None,
            }
        }
        _ => return None,
    };
    let external = &view.external;
    Some(crate::data::Link {
        uri: external.uri.clone(),
        title: external.title.clone(),
        description: external.description.clone(),
        thumb: external.thumb.clone(),
    })
}

/// Post cité par `post` (citation seule ou avec des images).
fn quote(post: &PostView, now: DateTime<Utc>) -> Option<Box<Post>> {
    use atrium_api::app::bsky::embed::record::ViewRecordRefs;
    let view = match post.embed.as_ref()? {
        Union::Refs(PostViewEmbedRefs::AppBskyEmbedRecordView(view)) => &**view,
        Union::Refs(PostViewEmbedRefs::AppBskyEmbedRecordWithMediaView(with_media)) => {
            &with_media.record
        }
        _ => return None,
    };
    let unavailable = |text: String| {
        let mut post = crate::data::Post::unavailable();
        post.text = text;
        Some(Box::new(post))
    };
    match &view.record {
        Union::Refs(ViewRecordRefs::ViewRecord(record)) => {
            let indexed = parse_datetime(record.indexed_at.as_str());
            let value = serde_json::to_value(&record.value).unwrap_or_default();
            let mut quoted = crate::data::Post::unavailable();
            quoted.uri = Some(record.uri.clone());
            quoted.cid = serde_json::to_value(&record.cid)
                .ok()
                .and_then(|cid| cid.as_str().map(str::to_owned));
            quoted.name = display_name(&record.author);
            quoted.handle = record.author.handle.as_str().to_owned();
            quoted.avatar = record.author.avatar.clone();
            quoted.time = indexed.map(|t| relative_time(t, now)).unwrap_or_default();
            quoted.date = indexed.map(full_date).unwrap_or_default();
            quoted.text = value
                .get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            quoted.replies = count(record.reply_count);
            quoted.reposts = count(record.repost_count);
            quoted.likes = count(record.like_count);
            quoted.labels = record
                .labels
                .iter()
                .flatten()
                .filter(|label| label.neg != Some(true))
                .map(|label| label.val.clone())
                .collect();
            Some(Box::new(quoted))
        }
        Union::Refs(ViewRecordRefs::ViewNotFound(_)) => {
            unavailable(gettext("Le post cité a été supprimé."))
        }
        Union::Refs(ViewRecordRefs::ViewBlocked(_)) => unavailable(gettext(
            "Le post cité n'est pas visible : un blocage l'empêche.",
        )),
        Union::Refs(ViewRecordRefs::ViewDetached(_)) => {
            unavailable(gettext("Son auteur a retiré la citation de ce post."))
        }
        _ => None,
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
        // Étiquettes de Bluesky et de l'auteur lui-même : la valeur seule ;
        // celles d'un service tiers : « service#valeur ».
        let src = label.src.as_str();
        let value = if src == crate::moderation::BLUESKY_LABELER || src == post.author.did.as_str()
        {
            label.val.clone()
        } else {
            format!("{src}{}{}", crate::moderation::LABELER_SEPARATOR, label.val)
        };
        if label.neg == Some(true) {
            labels.retain(|v| *v != value);
        } else if !labels.contains(&value) {
            labels.push(value);
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
    let pinned_here = matches!(
        &item.reason,
        Some(Union::Refs(FeedViewPostReasonRefs::ReasonPin(_)))
    );
    let replying_to = item.reply.as_ref().and_then(|reply| match &reply.parent {
        Union::Refs(ReplyRefParentRefs::PostView(parent)) => Some(display_name(&parent.author)),
        _ => None,
    });
    post.context = match (reposted_by, replying_to) {
        _ if pinned_here => Some(gettext("Épinglé")),
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
        facets: serde_json::to_value(&post.record)
            .map(|record| crate::richtext::from_record(&record))
            .unwrap_or_default(),
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

/// Raisons de notification gardées par le filtre « Mentions ».
pub const MENTION_REASONS: &[&str] = &["mention", "reply", "quote"];

async fn notifications(
    api: &crate::auth::Api,
    cursor: Option<String>,
    reasons: Option<&[&str]>,
) -> Result<Page, ApiError> {
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
                reasons: reasons.map(|r| r.iter().map(|r| (*r).to_owned()).collect()),
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
    Ok(Page {
        posts: grouped_notifications(&output.data.notifications, &posts, Utc::now()),
        cursor: output.data.cursor,
    })
}

/// Clé de regroupement : les j'aime, reposts et abonnements d'une page se
/// regroupent par raison et par post concerné.
fn group_key(notification: &Notification) -> Option<(String, String)> {
    match notification.reason.as_str() {
        "like" | "repost" | "like-via-repost" | "repost-via-repost" | "follow" => Some((
            notification.reason.clone(),
            subject_uri(notification).unwrap_or_default().to_owned(),
        )),
        _ => None,
    }
}

/// « Aya Koné et 3 autres ont aimé votre post ».
fn group_context(reason: &str, name: &str, others: usize) -> Option<String> {
    let n = others.min(u32::MAX as usize) as u32;
    let text = match reason {
        "like" => ngettext(
            "{name} et {n} autre ont aimé votre post",
            "{name} et {n} autres ont aimé votre post",
            n,
        ),
        "repost" => ngettext(
            "{name} et {n} autre ont reposté votre post",
            "{name} et {n} autres ont reposté votre post",
            n,
        ),
        "like-via-repost" => ngettext(
            "{name} et {n} autre ont aimé votre repost",
            "{name} et {n} autres ont aimé votre repost",
            n,
        ),
        "repost-via-repost" => ngettext(
            "{name} et {n} autre ont reposté votre repost",
            "{name} et {n} autres ont reposté votre repost",
            n,
        ),
        "follow" => ngettext(
            "{name} et {n} autre vous suivent",
            "{name} et {n} autres vous suivent",
            n,
        ),
        _ => return None,
    };
    Some(text.replace("{name}", name).replace("{n}", &n.to_string()))
}

/// Cartes d'une page de notifications, les j'aime, reposts et abonnements
/// d'un même post réunis en une carte : celle du plus récent, avec le
/// nombre des autres. Non lue si l'une d'elles ne l'est pas.
pub fn grouped_notifications(
    notifications: &[Notification],
    posts: &HashMap<String, PostView>,
    now: DateTime<Utc>,
) -> Vec<Post> {
    let mut cards: Vec<Post> = Vec::new();
    // Clé → (indice de la carte, nombre d'autres).
    let mut groups: HashMap<(String, String), (usize, usize)> = HashMap::new();
    for notification in notifications {
        let key = group_key(notification);
        if let Some(key) = &key
            && let Some((index, others)) = groups.get_mut(key)
        {
            *others += 1;
            let card = &mut cards[*index];
            card.unread |= !notification.is_read;
            card.context = group_context(&notification.reason, &card.name, *others);
            continue;
        }
        let Some(card) = post_from_notification(notification, posts, now) else {
            continue;
        };
        if let Some(key) = key {
            groups.insert(key, (cards.len(), 0));
        }
        cards.push(card);
    }
    cards
}

/// Nombre de notifications pas encore vues (`getUnreadCount`).
pub async fn unread_count(session: Arc<Session>) -> Result<u32, ApiError> {
    let api = session.api().await?;
    let output = Box::pin(
        api.api.app.bsky.notification.get_unread_count(
            atrium_api::app::bsky::notification::get_unread_count::ParametersData {
                priority: None,
                seen_at: None,
            }
            .into(),
        ),
    )
    .await
    .map_err(request_failed)?;
    Ok(output.data.count.clamp(0, i64::from(u32::MAX)) as u32)
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
            facets: serde_json::to_value(&notification.record)
                .map(|record| crate::richtext::from_record(&record))
                .unwrap_or_default(),
            avatar: None,
            media: None,
            replies: 0,
            reposts: 0,
            likes: 0,
            thread: Vec::new(),
            unread: false,
            labels: Vec::new(),
            pinned: false,
            quote: None,
            link: None,
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

/// Comptes dont l'identifiant ou le nom commence par `query`, pour
/// compléter une mention (`searchActorsTypeahead`).
pub async fn typeahead(session: Arc<Session>, query: String) -> Result<Vec<Actor>, ApiError> {
    let api = session.api().await?;
    let output = Box::pin(
        api.api.app.bsky.actor.search_actors_typeahead(
            atrium_api::app::bsky::actor::search_actors_typeahead::ParametersData {
                limit: 8.try_into().ok(),
                q: Some(query),
                term: None,
            }
            .into(),
        ),
    )
    .await
    .map_err(request_failed)?;
    Ok(output
        .data
        .actors
        .iter()
        .map(|actor| Actor {
            did: actor.did.as_str().to_owned(),
            handle: actor.handle.as_str().to_owned(),
            name: display_name(actor),
            bio: String::new(),
            avatar: actor.avatar.clone(),
            following: actor.viewer.as_ref().and_then(|v| v.following.clone()),
        })
        .collect())
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

pub(crate) fn invalid_actor() -> ApiError {
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
        known_followers: viewer
            .and_then(|v| v.known_followers.as_ref())
            .map(|known| {
                known
                    .followers
                    .iter()
                    .map(|f| name_or_handle(f.display_name.as_deref(), f.handle.as_str()))
                    .collect()
            })
            .unwrap_or_default(),
        known_count: viewer
            .and_then(|v| v.known_followers.as_ref())
            .map(|known| known.count.max(0) as u64)
            .unwrap_or(0),
        activity: viewer
            .and_then(|v| v.activity_subscription.as_ref())
            .map(|a| (a.post, a.reply))
            .unwrap_or((false, false)),
        associated: profile
            .associated
            .as_ref()
            .map(|a| (count(a.feedgens), count(a.lists), count(a.starter_packs)))
            .unwrap_or_default(),
        labeler: profile
            .associated
            .as_ref()
            .and_then(|a| a.labeler)
            .unwrap_or(false),
    }
}

/// Demande (ou arrête) les notifications des posts et des réponses d'un
/// compte (`putActivitySubscription`).
pub async fn set_activity(
    session: Arc<Session>,
    did: String,
    post: bool,
    reply: bool,
) -> Result<(), ApiError> {
    use atrium_api::app::bsky::notification::{defs, put_activity_subscription};
    let api = session.api().await?;
    api.api
        .app
        .bsky
        .notification
        .put_activity_subscription(
            put_activity_subscription::InputData {
                activity_subscription: defs::ActivitySubscriptionData { post, reply }.into(),
                subject: did.parse().map_err(|_| invalid_actor())?,
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(())
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

pub(crate) fn list_from_view(list: &atrium_api::app::bsky::graph::defs::ListView) -> List {
    let purpose = serde_json::to_value(&list.purpose).unwrap_or_default();
    List {
        uri: list.uri.clone(),
        name: list.name.clone(),
        description: list.description.clone().unwrap_or_default(),
        avatar: list.avatar.clone(),
        members: list.list_item_count.unwrap_or(0) as u64,
        moderation: purpose.as_str() == Some("app.bsky.graph.defs#modlist"),
        creator: list.creator.did.as_str().to_owned(),
        muted: list.viewer.as_ref().and_then(|v| v.muted).unwrap_or(false),
        blocked: list.viewer.as_ref().and_then(|v| v.blocked.clone()),
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

/// Liste de comptes : abonnés ou abonnements d'un compte (DID ou
/// identifiant), comptes qui ont aimé ou reposté un post (adresse).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum People {
    Followers(String),
    Follows(String),
    Likers(String),
    Reposters(String),
    /// Comptes que je suis qui suivent ce compte.
    Known(String),
}

impl People {
    pub fn title(&self) -> String {
        match self {
            People::Followers(_) => gettext("Abonnés"),
            People::Follows(_) => gettext("Abonnements"),
            People::Likers(_) => pgettext("comptes qui ont aimé un post", "J'aime"),
            People::Reposters(_) => gettext("Reposts"),
            People::Known(_) => gettext("Abonnés que vous connaissez"),
        }
    }

    pub fn empty(&self) -> String {
        match self {
            People::Followers(_) => gettext("Aucun abonné pour l'instant."),
            People::Follows(_) => gettext("Aucun abonnement pour l'instant."),
            People::Likers(_) => gettext("Personne n'a encore aimé ce post."),
            People::Reposters(_) => gettext("Personne n'a encore reposté ce post."),
            People::Known(_) => gettext("Aucun compte que vous suivez ne suit ce compte."),
        }
    }
}

/// Une page de la liste, par 50, avec la suite.
pub async fn people(
    session: Arc<Session>,
    people: People,
    cursor: Option<String>,
) -> Result<(Vec<Actor>, Option<String>), ApiError> {
    use atrium_api::app::bsky::feed::{get_likes, get_reposted_by};
    match people {
        People::Known(actor) => {
            use atrium_api::app::bsky::graph::get_known_followers;
            let api = session.api().await?;
            let output = Box::pin(
                api.api.app.bsky.graph.get_known_followers(
                    get_known_followers::ParametersData {
                        actor: actor.parse().map_err(|_| invalid_actor())?,
                        cursor,
                        limit: 50.try_into().ok(),
                    }
                    .into(),
                ),
            )
            .await
            .map_err(request_failed)?;
            Ok((
                output.data.followers.iter().map(actor_from_view).collect(),
                output.data.cursor,
            ))
        }
        People::Followers(actor) => Box::pin(connections(session, actor, true, cursor)).await,
        People::Follows(actor) => Box::pin(connections(session, actor, false, cursor)).await,
        People::Likers(uri) => {
            let api = session.api().await?;
            let output = Box::pin(
                api.api.app.bsky.feed.get_likes(
                    get_likes::ParametersData {
                        cid: None,
                        cursor,
                        limit: 50.try_into().ok(),
                        uri,
                    }
                    .into(),
                ),
            )
            .await
            .map_err(request_failed)?;
            Ok((
                output
                    .data
                    .likes
                    .iter()
                    .map(|like| actor_from_view(&like.actor))
                    .collect(),
                output.data.cursor,
            ))
        }
        People::Reposters(uri) => {
            let api = session.api().await?;
            let output = Box::pin(
                api.api.app.bsky.feed.get_reposted_by(
                    get_reposted_by::ParametersData {
                        cid: None,
                        cursor,
                        limit: 50.try_into().ok(),
                        uri,
                    }
                    .into(),
                ),
            )
            .await
            .map_err(request_failed)?;
            Ok((
                output
                    .data
                    .reposted_by
                    .iter()
                    .map(actor_from_view)
                    .collect(),
                output.data.cursor,
            ))
        }
    }
}

/// Abonnés (`followers` vrai) ou abonnements d'un compte, par pages de 50.
async fn connections(
    session: Arc<Session>,
    actor: String,
    followers: bool,
    cursor: Option<String>,
) -> Result<(Vec<Actor>, Option<String>), ApiError> {
    use atrium_api::app::bsky::graph::{get_followers, get_follows};
    let api = session.api().await?;
    let actor: atrium_api::types::string::AtIdentifier =
        actor.parse().map_err(|_| invalid_actor())?;
    let graph = &api.api.app.bsky.graph;
    let (views, cursor) = if followers {
        let output = graph
            .get_followers(
                get_followers::ParametersData {
                    actor,
                    cursor,
                    limit: 50.try_into().ok(),
                    sort: None,
                }
                .into(),
            )
            .await
            .map_err(request_failed)?;
        (output.data.followers, output.data.cursor)
    } else {
        let output = graph
            .get_follows(
                get_follows::ParametersData {
                    actor,
                    cursor,
                    limit: 50.try_into().ok(),
                    sort: None,
                }
                .into(),
            )
            .await
            .map_err(request_failed)?;
        (output.data.follows, output.data.cursor)
    };
    Ok((views.iter().map(actor_from_view).collect(), cursor))
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

/// Qui peut répondre à un nouveau post (`app.bsky.feed.threadgate`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReplyRule {
    #[default]
    Everyone,
    Following,
    Followers,
    Mentioned,
    Nobody,
}

impl ReplyRule {
    pub const ALL: [ReplyRule; 5] = [
        ReplyRule::Everyone,
        ReplyRule::Following,
        ReplyRule::Followers,
        ReplyRule::Mentioned,
        ReplyRule::Nobody,
    ];

    pub fn id(self) -> &'static str {
        match self {
            ReplyRule::Everyone => "everyone",
            ReplyRule::Following => "following",
            ReplyRule::Followers => "followers",
            ReplyRule::Mentioned => "mentioned",
            ReplyRule::Nobody => "nobody",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|rule| rule.id() == id)
    }

    /// Libellé court, pour le menu et le bouton.
    pub fn title(self) -> String {
        match self {
            ReplyRule::Everyone => gettext("Tout le monde"),
            ReplyRule::Following => gettext("Comptes que vous suivez"),
            ReplyRule::Followers => gettext("Vos abonnés"),
            ReplyRule::Mentioned => gettext("Comptes mentionnés"),
            ReplyRule::Nobody => gettext("Personne"),
        }
    }
}

/// Enregistrements qui accompagnent un nouveau post, avec la même clé :
/// qui peut répondre, et l'interdiction de le citer.
pub fn gate_records(
    post_uri: &str,
    rule: ReplyRule,
    no_quotes: bool,
    created_at: &str,
) -> Vec<(&'static str, serde_json::Value)> {
    use serde_json::json;
    let mut records = Vec::new();
    let allow = match rule {
        ReplyRule::Everyone => None,
        ReplyRule::Following => Some(vec![
            json!({"$type": "app.bsky.feed.threadgate#followingRule"}),
        ]),
        ReplyRule::Followers => Some(vec![
            json!({"$type": "app.bsky.feed.threadgate#followerRule"}),
        ]),
        ReplyRule::Mentioned => Some(vec![
            json!({"$type": "app.bsky.feed.threadgate#mentionRule"}),
        ]),
        ReplyRule::Nobody => Some(Vec::new()),
    };
    if let Some(allow) = allow {
        records.push((
            "app.bsky.feed.threadgate",
            json!({
                "$type": "app.bsky.feed.threadgate",
                "post": post_uri,
                "allow": allow,
                "createdAt": created_at,
            }),
        ));
    }
    if no_quotes {
        records.push((
            "app.bsky.feed.postgate",
            json!({
                "$type": "app.bsky.feed.postgate",
                "post": post_uri,
                "embeddingRules": [{"$type": "app.bsky.feed.postgate#disableRule"}],
                "createdAt": created_at,
            }),
        ));
    }
    records
}

/// Post publié. `gates_failed` : les réglages de réponse ou de citation
/// n'ont pas pu être enregistrés, alors que le post l'a été.
pub struct Published {
    pub post: StrongRef,
    pub gates_failed: bool,
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
    /// Post cité.
    pub quote: Option<StrongRef>,
    /// Qui peut répondre (sans effet sur une réponse).
    pub reply_rule: ReplyRule,
    /// Interdire de citer ce post.
    pub no_quotes: bool,
    /// Étiquette de contenu posée par l'auteur sur ses images (« nudity »,
    /// « sexual », « porn », « graphic-media »).
    pub self_label: Option<&'static str>,
    /// Lien joint, avec son aperçu (sans effet avec des images).
    pub link: Option<NewLink>,
    /// Vidéo jointe (seule, sans images).
    pub video: Option<NewVideo>,
}

/// Vidéo jointe à un nouveau post.
#[derive(Debug, Clone, Default)]
pub struct NewVideo {
    pub bytes: Vec<u8>,
    /// Type MIME du fichier (« video/mp4 »…).
    pub mime: String,
    pub alt: String,
    /// Largeur et hauteur, si le lecteur a pu les lire.
    pub size: Option<(u32, u32)>,
    /// Présentation suggérée : « gif » pour un GIF animé (en boucle, sans
    /// commandes).
    pub presentation: Option<&'static str>,
    /// Blob de la vidéo, une fois traitée par le service vidéo.
    pub blob: Option<serde_json::Value>,
}

/// Service qui reçoit et convertit les vidéos de Bluesky.
const VIDEO_SERVICE: &str = "https://video.bsky.app";

/// Lien joint à un nouveau post.
#[derive(Debug, Clone, Default)]
pub struct NewLink {
    pub uri: String,
    pub title: String,
    pub description: String,
    pub thumb: Option<crate::photo::Prepared>,
    /// Blob de l'image, une fois envoyée.
    pub thumb_blob: Option<serde_json::Value>,
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
    if let Some(value) = post.self_label {
        record["labels"] = json!({
            "$type": "com.atproto.label.defs#selfLabels",
            "values": [{"val": value}],
        });
    }
    if !post.langs.is_empty() {
        record["langs"] = json!(post.langs);
    }
    if !facets.is_empty() {
        record["facets"] = json!(facets);
    }
    if let Some(video) = &post.video
        && let Some(blob) = &video.blob
    {
        let mut embed = json!({"$type": "app.bsky.embed.video", "video": blob});
        if !video.alt.is_empty() {
            embed["alt"] = json!(video.alt);
        }
        if let Some((width, height)) = video.size {
            embed["aspectRatio"] = json!({"width": width, "height": height});
        }
        if let Some(presentation) = video.presentation {
            embed["presentation"] = json!(presentation);
        }
        record["embed"] = embed;
    } else if !blobs.is_empty() {
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
    } else if let Some(link) = &post.link {
        let mut external = json!({
            "uri": link.uri,
            "title": link.title,
            "description": link.description,
        });
        if let Some(blob) = &link.thumb_blob {
            external["thumb"] = blob.clone();
        }
        record["embed"] = json!({"$type": "app.bsky.embed.external", "external": external});
    }
    if let Some(quote) = &post.quote {
        let quoted = json!({
            "$type": "app.bsky.embed.record",
            "record": {"uri": quote.uri, "cid": quote.cid},
        });
        record["embed"] = match record.get("embed").cloned() {
            Some(media) => json!({
                "$type": "app.bsky.embed.recordWithMedia",
                "record": quoted,
                "media": media,
            }),
            None => quoted,
        };
    }
    if let Some((root, parent)) = &post.reply {
        record["reply"] = json!({
            "root": {"uri": root.uri, "cid": root.cid},
            "parent": {"uri": parent.uri, "cid": parent.cid},
        });
    }
    record
}

/// Publie un post, puis ses réglages de réponse et de citation.
pub async fn publish(session: Arc<Session>, mut post: NewPost) -> Result<Published, ApiError> {
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
    // Vidéo : envoyée au service vidéo, qui la convertit et rend son blob.
    if let Some(video) = post.video.as_mut()
        && video.blob.is_none()
    {
        video.blob = Some(upload_video(&api, &session.account.did, video).await?);
    }
    // Image de l'aperçu du lien : sans elle, la carte reste valable.
    if post.images.is_empty()
        && post.video.is_none()
        && let Some(link) = post.link.as_mut()
        && let Some(thumb) = &link.thumb
    {
        match api
            .api
            .com
            .atproto
            .repo
            .upload_blob(thumb.bytes.clone())
            .await
        {
            Ok(output) => link.thumb_blob = serde_json::to_value(&output.data.blob).ok(),
            Err(error) => glib_debug(&format!("aperçu du lien : {error}")),
        }
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
    let published = StrongRef {
        uri: output.data.uri,
        cid: serde_json::to_value(&output.data.cid)
            .ok()
            .and_then(|cid| cid.as_str().map(str::to_owned))
            .unwrap_or_default(),
    };
    // Une réponse suit les réglages du début de la discussion.
    let rule = if post.reply.is_some() {
        ReplyRule::Everyone
    } else {
        post.reply_rule
    };
    let rkey = parse_record_uri(&published.uri).map(|(_, _, rkey)| rkey.to_owned());
    let mut gates_failed = false;
    for (collection, record) in
        gate_records(&published.uri, rule, post.no_quotes, created_at.as_str())
    {
        let (Some(rkey), Ok(record)) = (rkey.clone(), serde_json::from_value(record)) else {
            gates_failed = true;
            continue;
        };
        let result = Box::pin(
            api.api.com.atproto.repo.create_record(
                create_record::InputData {
                    collection: Nsid::new(collection.to_owned()).map_err(|_| invalid_post())?,
                    record,
                    repo: Did::new(session.account.did.clone())
                        .map_err(|_| invalid_post())?
                        .into(),
                    rkey: atrium_api::types::string::RecordKey::new(rkey).ok(),
                    swap_commit: None,
                    validate: None,
                }
                .into(),
            ),
        )
        .await;
        if let Err(error) = result {
            glib_debug(&format!("{collection} : {error}"));
            gates_failed = true;
        }
    }
    Ok(Published {
        post: published,
        gates_failed,
    })
}

/// Envoie une vidéo au service vidéo avec un jeton de service du PDS, puis
/// attend la fin de sa conversion. Rend le blob à citer dans le post.
async fn upload_video(
    api: &crate::auth::Api,
    did: &str,
    video: &NewVideo,
) -> Result<serde_json::Value, ApiError> {
    use atrium_api::com::atproto::server::get_service_auth;
    use atrium_api::types::string::Nsid;

    let failed = |detail: String| {
        glib_debug(&format!("vidéo : {detail}"));
        ApiError::new(gettext("La vidéo n'a pas pu être envoyée."))
    };
    // Le jeton est destiné au PDS du compte : le service vidéo y dépose
    // le blob en son nom.
    let server = api
        .api
        .com
        .atproto
        .server
        .describe_server()
        .await
        .map_err(|e| failed(e.to_string()))?;
    let token = api
        .api
        .com
        .atproto
        .server
        .get_service_auth(
            get_service_auth::ParametersData {
                aud: server.data.did.as_str().to_owned(),
                exp: Some(chrono::Utc::now().timestamp() + 30 * 60),
                lxm: Nsid::new("com.atproto.repo.uploadBlob".to_owned()).ok(),
            }
            .into(),
        )
        .await
        .map_err(|e| failed(e.to_string()))?
        .data
        .token;
    let client = reqwest::Client::new();
    let name = format!("{}.mp4", chrono::Utc::now().timestamp_millis());
    let response = client
        .post(format!(
            "{VIDEO_SERVICE}/xrpc/app.bsky.video.uploadVideo?did={did}&name={name}"
        ))
        .bearer_auth(token)
        .header("Content-Type", &video.mime)
        .body(video.bytes.clone())
        .send()
        .await
        .map_err(|e| failed(e.to_string()))?;
    let status = response.status();
    let body = response.bytes().await.map_err(|e| failed(e.to_string()))?;
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
    // Une vidéo déjà envoyée renvoie une erreur avec la tâche existante.
    let mut job = job_status(&body);
    let Some(job_id) = job["jobId"].as_str().map(str::to_owned) else {
        return Err(failed(format!("{status} {body}")));
    };
    for _ in 0..300 {
        if let Some(blob) = job.get("blob") {
            return Ok(blob.clone());
        }
        match job["state"].as_str() {
            Some("JOB_STATE_FAILED") => {
                let detail = job["error"].as_str().or(job["message"].as_str());
                glib_debug(&format!("vidéo : {job}"));
                return Err(ApiError::new(match detail {
                    Some(detail) => gettext("La vidéo a été refusée : {}").replace("{}", detail),
                    None => gettext("La vidéo n'a pas pu être convertie."),
                }));
            }
            _ => tokio::time::sleep(std::time::Duration::from_secs(2)).await,
        }
        let body = client
            .get(format!(
                "{VIDEO_SERVICE}/xrpc/app.bsky.video.getJobStatus?jobId={job_id}"
            ))
            .send()
            .await
            .map_err(|e| failed(e.to_string()))?
            .bytes()
            .await
            .map_err(|e| failed(e.to_string()))?;
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        job = job_status(&body);
    }
    Err(ApiError::new(gettext(
        "La conversion de la vidéo prend trop de temps.",
    )))
}

/// L'état d'une tâche du service vidéo, sous `jobStatus` ou à la racine.
fn job_status(body: &serde_json::Value) -> serde_json::Value {
    body.get("jobStatus").unwrap_or(body).clone()
}

fn glib_debug(message: &str) {
    gtk::glib::g_debug!("tsubame", "{}", message);
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
            day_month(local.day(), local.month())
        } else {
            pgettext("jour, mois abrégé et année", "{day} {month} {year}")
                .replace("{day}", &local.day().to_string())
                .replace("{month}", &month(local.month()))
                .replace("{year}", &local.year().to_string())
        }
    }
}

/// Date complète du volet de discussion : « 20 nov. 2026, 14:32 ».
pub fn full_date(then: DateTime<FixedOffset>) -> String {
    let local = then.with_timezone(&Local);
    pgettext("date et heure d'un post", "{day} {month} {year}, {time}")
        .replace("{day}", &local.day().to_string())
        .replace("{month}", &month(local.month()))
        .replace("{year}", &local.year().to_string())
        .replace(
            "{time}",
            &format!("{:02}:{:02}", local.hour(), local.minute()),
        )
}

/// « 20 nov. » (ou « Nov 20 » selon la langue).
pub fn day_month(day: u32, month_number: u32) -> String {
    pgettext("jour et mois abrégé", "{day} {month}")
        .replace("{day}", &day.to_string())
        .replace("{month}", &month(month_number))
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
                video: None,
                images: vec![
                    crate::data::Image {
                        thumb: "https://cdn.example/thumb.jpg".into(),
                        fullsize: "https://cdn.example/full.jpg".into(),
                        alt: "Capture du sélecteur de fichiers".into(),
                    },
                    crate::data::Image {
                        thumb: "https://cdn.example/thumb2.jpg".into(),
                        fullsize: "https://cdn.example/full2.jpg".into(),
                        alt: String::new(),
                    },
                ],
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
                    "labels": [{"src": "did:plc:ar7c4by46qjdydhdevvrndac", "uri": "did:plc:r", "val": "nudity", "cts": "2026-11-20T14:56:00.000Z"}]},
                "record": {"$type": "app.bsky.feed.post", "text": "Bonjour", "createdAt": "2026-11-20T14:56:00.000Z"},
                "labels": [
                    {"src": "did:plc:ar7c4by46qjdydhdevvrndac", "uri": "at://did:plc:r/app.bsky.feed.post/1", "val": "graphic-media", "cts": "2026-11-20T14:56:00.000Z"},
                    {"src": "did:plc:ar7c4by46qjdydhdevvrndac", "uri": "at://did:plc:r/app.bsky.feed.post/1", "val": "porn", "cts": "2026-11-20T14:56:00.000Z"},
                    {"src": "did:plc:ar7c4by46qjdydhdevvrndac", "uri": "at://did:plc:r/app.bsky.feed.post/1", "val": "porn", "neg": true, "cts": "2026-11-20T14:57:00.000Z"},
                    {"src": "did:plc:veille", "uri": "at://did:plc:r/app.bsky.feed.post/1", "val": "spoiler", "cts": "2026-11-20T14:57:00.000Z"}
                ],
                "indexedAt": "2026-11-20T14:56:00.000Z"
            }"#,
        )
        .unwrap();
        assert_eq!(
            post_from_view(&post, now()).labels,
            vec![
                "graphic-media".to_owned(),
                "did:plc:veille#spoiler".to_owned(),
                "nudity".to_owned()
            ]
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
    fn likes_and_follows_are_grouped() {
        let subject = Some("at://did:plc:moi/app.bsky.feed.post/1");
        let like_record = r#"{"$type": "app.bsky.feed.like", "subject": {"uri": "at://did:plc:moi/app.bsky.feed.post/1", "cid": "x"}, "createdAt": "2026-11-20T14:20:00.000Z"}"#;
        let follow_record = r#"{"$type": "app.bsky.graph.follow", "subject": "did:plc:moi", "createdAt": "2026-11-20T14:20:00.000Z"}"#;
        let reply_record = r#"{"$type": "app.bsky.feed.post", "text": "Bravo", "createdAt": "2026-11-20T14:20:00.000Z"}"#;
        let mut read_like = notification(
            "like",
            "at://did:plc:pierre/app.bsky.feed.like/2",
            subject,
            like_record,
        );
        read_like.is_read = true;
        let notifications = vec![
            notification(
                "like",
                "at://did:plc:pierre/app.bsky.feed.like/1",
                subject,
                like_record,
            ),
            notification(
                "follow",
                "at://did:plc:pierre/app.bsky.graph.follow/1",
                None,
                follow_record,
            ),
            notification(
                "reply",
                "at://did:plc:pierre/app.bsky.feed.post/r",
                None,
                reply_record,
            ),
            read_like.clone(),
            read_like,
            notification(
                "follow",
                "at://did:plc:pierre/app.bsky.graph.follow/2",
                None,
                follow_record,
            ),
        ];
        let posts = HashMap::from([(my_post().uri.clone(), my_post())]);
        let cards = grouped_notifications(&notifications, &posts, now());
        assert_eq!(cards.len(), 3);
        assert_eq!(
            cards[0].context.as_deref(),
            Some("Pierre Delmas et 2 autres ont aimé votre post")
        );
        assert!(cards[0].unread);
        assert_eq!(
            cards[1].context.as_deref(),
            Some("Pierre Delmas et 1 autre vous suivent")
        );
        // Les réponses restent chacune sur sa carte.
        assert_eq!(cards[2].context.as_deref(), Some("a répondu à votre post"));
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
            "repost": null,
            "pinned": true
        });
        item["reason"] = serde_json::json!({"$type": "app.bsky.feed.defs#reasonPin"});
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
        assert!(post.pinned);
        assert_eq!(post.context.as_deref(), Some("Épinglé"));
    }

    const CID: &str = "bafyreie5737gdxlw5i64vzichcalba3z2v5n6icifvx5xytvske7mr3hpm";

    #[test]
    fn self_labels_go_in_the_record() {
        let post = NewPost {
            text: "Photo".into(),
            langs: Vec::new(),
            reply: None,
            quote: None,
            reply_rule: ReplyRule::Everyone,
            no_quotes: false,
            self_label: Some("graphic-media"),
            video: None,
            link: None,
            images: Vec::new(),
        };
        let record = post_record(&post, &HashMap::new(), "2026-11-20T15:00:00.000Z", &[]);
        assert_eq!(record["labels"]["values"][0]["val"], "graphic-media");
        let _: atrium_api::app::bsky::feed::post::Record = serde_json::from_value(record).unwrap();
    }

    #[test]
    fn videos_become_video_embeds() {
        let post = NewPost {
            text: String::new(),
            langs: Vec::new(),
            reply: None,
            quote: None,
            reply_rule: ReplyRule::Everyone,
            no_quotes: false,
            self_label: None,
            video: Some(NewVideo {
                alt: "Un train".into(),
                size: Some((1920, 1080)),
                presentation: Some("gif"),
                blob: Some(serde_json::json!({
                    "$type": "blob", "ref": {"$link": CID}, "mimeType": "video/mp4", "size": 1
                })),
                ..NewVideo::default()
            }),
            link: None,
            images: Vec::new(),
        };
        let record = post_record(&post, &HashMap::new(), "2026-11-20T15:00:00.000Z", &[]);
        assert_eq!(record["embed"]["$type"], "app.bsky.embed.video");
        assert_eq!(record["embed"]["alt"], "Un train");
        assert_eq!(record["embed"]["aspectRatio"]["width"], 1920);
        assert_eq!(record["embed"]["presentation"], "gif");
        let _: atrium_api::app::bsky::feed::post::Record = serde_json::from_value(record).unwrap();
    }

    #[test]
    fn video_jobs_are_read_at_both_levels() {
        let nested = serde_json::json!({"jobStatus": {"jobId": "a", "state": "JOB_STATE_CREATED"}});
        assert_eq!(job_status(&nested)["jobId"], "a");
        let flat = serde_json::json!({"jobId": "b", "error": "already_exists"});
        assert_eq!(job_status(&flat)["jobId"], "b");
    }

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
            quote: None,
            reply_rule: ReplyRule::Everyone,
            no_quotes: false,
            self_label: None,
            video: None,
            link: None,
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
            quote: None,
            reply_rule: ReplyRule::Everyone,
            no_quotes: false,
            self_label: None,
            video: None,
            link: None,
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
    fn reply_and_quote_rules_become_gates() {
        let uri = "at://did:plc:moi/app.bsky.feed.post/3k";
        let at = "2026-11-20T15:00:00.000Z";
        assert!(gate_records(uri, ReplyRule::Everyone, false, at).is_empty());
        let gates = gate_records(uri, ReplyRule::Nobody, true, at);
        assert_eq!(gates.len(), 2);
        assert_eq!(gates[0].0, "app.bsky.feed.threadgate");
        assert_eq!(gates[0].1["allow"], serde_json::json!([]));
        assert_eq!(gates[0].1["post"], uri);
        assert_eq!(gates[1].0, "app.bsky.feed.postgate");
        assert_eq!(
            gates[1].1["embeddingRules"][0]["$type"],
            "app.bsky.feed.postgate#disableRule"
        );
        let gates = gate_records(uri, ReplyRule::Followers, false, at);
        assert_eq!(
            gates[0].1["allow"][0]["$type"],
            "app.bsky.feed.threadgate#followerRule"
        );
        for rule in ReplyRule::ALL {
            assert_eq!(ReplyRule::from_id(rule.id()), Some(rule));
            for (collection, record) in gate_records(uri, rule, true, at) {
                match collection {
                    "app.bsky.feed.threadgate" => {
                        let _: atrium_api::app::bsky::feed::threadgate::Record =
                            serde_json::from_value(record).unwrap();
                    }
                    _ => {
                        let _: atrium_api::app::bsky::feed::postgate::Record =
                            serde_json::from_value(record).unwrap();
                    }
                }
            }
        }
    }

    #[test]
    fn links_are_embedded_with_their_preview() {
        let mut post = NewPost {
            text: "À lire : https://example.org/billet".into(),
            langs: vec!["fr".into()],
            reply: None,
            quote: None,
            reply_rule: ReplyRule::Everyone,
            no_quotes: false,
            self_label: None,
            video: None,
            images: Vec::new(),
            link: Some(NewLink {
                uri: "https://example.org/billet".into(),
                title: "Un billet".into(),
                description: "Sa description".into(),
                thumb: None,
                thumb_blob: Some(serde_json::json!({
                    "$type": "blob", "ref": {"$link": CID}, "mimeType": "image/jpeg", "size": 1
                })),
            }),
        };
        let record = post_record(&post, &HashMap::new(), "2026-11-20T15:00:00.000Z", &[]);
        assert_eq!(record["embed"]["$type"], "app.bsky.embed.external");
        assert_eq!(record["embed"]["external"]["title"], "Un billet");
        assert_eq!(record["embed"]["external"]["thumb"]["size"], 1);
        let _: atrium_api::app::bsky::feed::post::Record = serde_json::from_value(record).unwrap();
        // Avec une citation : lien et citation ensemble.
        post.quote = Some(StrongRef {
            uri: "at://did:plc:lucie/app.bsky.feed.post/1".into(),
            cid: CID.into(),
        });
        let record = post_record(&post, &HashMap::new(), "2026-11-20T15:00:00.000Z", &[]);
        assert_eq!(record["embed"]["$type"], "app.bsky.embed.recordWithMedia");
        assert_eq!(record["embed"]["media"]["$type"], "app.bsky.embed.external");
        let _: atrium_api::app::bsky::feed::post::Record = serde_json::from_value(record).unwrap();
    }

    #[test]
    fn quotes_are_embedded_alone_or_with_images() {
        let quoted = StrongRef {
            uri: "at://did:plc:lucie/app.bsky.feed.post/1".into(),
            cid: CID.into(),
        };
        let mut post = NewPost {
            text: "Bien vu".into(),
            langs: vec!["fr".into()],
            reply: None,
            quote: Some(quoted),
            reply_rule: ReplyRule::Everyone,
            no_quotes: false,
            self_label: None,
            video: None,
            link: None,
            images: Vec::new(),
        };
        let record = post_record(&post, &HashMap::new(), "2026-11-20T15:00:00.000Z", &[]);
        assert_eq!(record["embed"]["$type"], "app.bsky.embed.record");
        assert_eq!(
            record["embed"]["record"]["uri"],
            "at://did:plc:lucie/app.bsky.feed.post/1"
        );
        let _: atrium_api::app::bsky::feed::post::Record = serde_json::from_value(record).unwrap();

        post.images = vec![NewImage {
            prepared: crate::photo::Prepared {
                bytes: Vec::new(),
                mime: "image/jpeg",
                width: 10,
                height: 10,
            },
            alt: String::new(),
        }];
        let blob = serde_json::json!({
            "$type": "blob", "ref": {"$link": CID}, "mimeType": "image/jpeg", "size": 1
        });
        let record = post_record(&post, &HashMap::new(), "2026-11-20T15:00:00.000Z", &[blob]);
        let embed = &record["embed"];
        assert_eq!(embed["$type"], "app.bsky.embed.recordWithMedia");
        assert_eq!(embed["record"]["$type"], "app.bsky.embed.record");
        assert_eq!(embed["record"]["record"]["cid"], CID);
        assert_eq!(embed["media"]["$type"], "app.bsky.embed.images");
        let _: atrium_api::app::bsky::feed::post::Record = serde_json::from_value(record).unwrap();
    }

    #[test]
    fn videos_keep_their_playlist() {
        let mut item: serde_json::Value = serde_json::from_str(FEED_ITEM).unwrap();
        item["post"]["embed"] = serde_json::json!({
            "$type": "app.bsky.embed.video#view",
            "cid": CID,
            "playlist": "https://video.example/watch/playlist.m3u8",
            "thumbnail": "https://video.example/watch/thumbnail.jpg",
            "alt": "Démonstration",
            "aspectRatio": {"width": 1920, "height": 1080}
        });
        let item: FeedViewPost = serde_json::from_value(item).unwrap();
        let media = post_from_feed(&item, now()).media.unwrap();
        assert_eq!(
            media.video.as_deref(),
            Some("https://video.example/watch/playlist.m3u8")
        );
        assert_eq!(
            media.thumb.as_deref(),
            Some("https://video.example/watch/thumbnail.jpg")
        );
        assert_eq!(media.aspect_ratio, Some((1920, 1080)));
    }

    #[test]
    fn external_links_are_read() {
        let mut item: serde_json::Value = serde_json::from_str(FEED_ITEM).unwrap();
        item["post"]["embed"] = serde_json::json!({
            "$type": "app.bsky.embed.external#view",
            "external": {
                "uri": "https://example.org/billet",
                "title": "Un billet",
                "description": "Sa description",
                "thumb": "https://cdn.example/link.jpg"
            }
        });
        let item: FeedViewPost = serde_json::from_value(item).unwrap();
        let link = post_from_feed(&item, now()).link.unwrap();
        assert_eq!(link.uri, "https://example.org/billet");
        assert_eq!(link.title, "Un billet");
        assert_eq!(link.thumb.as_deref(), Some("https://cdn.example/link.jpg"));
    }

    #[test]
    fn quoted_posts_are_read() {
        let mut item: serde_json::Value = serde_json::from_str(FEED_ITEM).unwrap();
        item["post"]["embed"] = serde_json::json!({
            "$type": "app.bsky.embed.record#view",
            "record": {
                "$type": "app.bsky.embed.record#viewRecord",
                "uri": "at://did:plc:lucie/app.bsky.feed.post/1",
                "cid": CID,
                "author": {"did": "did:plc:lucie", "handle": "lucie.example.social", "displayName": "Lucie Morel"},
                "value": {"$type": "app.bsky.feed.post", "text": "GNOME 49 est sorti", "createdAt": "2026-11-20T14:00:00.000Z"},
                "indexedAt": "2026-11-20T14:00:00.000Z"
            }
        });
        let item: FeedViewPost = serde_json::from_value(item.clone()).unwrap();
        let quoted = post_from_feed(&item, now()).quote.unwrap();
        assert_eq!(quoted.name, "Lucie Morel");
        assert_eq!(quoted.text, "GNOME 49 est sorti");
        assert_eq!(quoted.cid.as_deref(), Some(CID));

        let mut item: serde_json::Value = serde_json::from_str(FEED_ITEM).unwrap();
        item["post"]["embed"] = serde_json::json!({
            "$type": "app.bsky.embed.record#view",
            "record": {
                "$type": "app.bsky.embed.record#viewNotFound",
                "uri": "at://did:plc:lucie/app.bsky.feed.post/1",
                "notFound": true
            }
        });
        let item: FeedViewPost = serde_json::from_value(item).unwrap();
        let quoted = post_from_feed(&item, now()).quote.unwrap();
        assert_eq!(quoted.uri, None);
        assert_eq!(quoted.text, "Le post cité a été supprimé.");
    }

    #[test]
    fn plain_posts_have_no_facets_nor_reply() {
        let post = NewPost {
            text: "Bonjour".into(),
            langs: Vec::new(),
            reply: None,
            quote: None,
            reply_rule: ReplyRule::Everyone,
            no_quotes: false,
            self_label: None,
            video: None,
            link: None,
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
