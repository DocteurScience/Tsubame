//! Messagerie (`chat.bsky.convo`), par le service de chat de Bluesky.
//!
//! Les requêtes passent par le serveur du compte, qui les relaie au
//! service de chat (en-tête `atproto-proxy`). La session OAuth doit avoir
//! la portée `transition:chat.bsky`, demandée à la connexion.

use std::sync::Arc;

use atrium_api::types::string::Did;
use chrono::{DateTime, Datelike, Local, Timelike, Utc};
use gettextrs::gettext;
use serde_json::Value;

use crate::api::{self, ApiError, name_or_handle, parse_datetime, request_failed};
use crate::auth::Session;

const CHAT_SERVICE: &str = "did:web:api.bsky.chat";
const CHAT_SERVICE_TYPE: &str = "bsky_chat";

/// Conversation dans la liste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Convo {
    pub id: String,
    /// Nom du groupe, ou des autres membres.
    pub title: String,
    /// Identifiant de l'autre membre (conversation à deux).
    pub handle: Option<String>,
    pub avatar: Option<String>,
    pub last_text: String,
    pub last_time: String,
    pub unread: u32,
    /// Demande de message pas encore acceptée.
    pub request: bool,
}

/// Message d'une conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub text: String,
    pub mine: bool,
    pub sender: String,
    /// Heure (« 14:40 ») et jour (« Aujourd'hui », « Hier », « 18 nov. »).
    pub time: String,
    pub day: String,
    /// Envoi échoué (message resté sur cet appareil).
    pub failed: bool,
}

/// Messages d'une page, du plus ancien au plus récent, et le curseur des
/// messages plus anciens s'il en reste.
pub type MessagePage = (Vec<Message>, Option<String>);

/// Préfixe des messages en cours d'envoi, pas encore connus du service.
pub const LOCAL_ID: &str = "local:";

/// Fusionne une page récente (`latest`) avec les messages affichés : ceux
/// qu'elle ne contient pas restent devant (plus anciens) ; ceux en cours
/// d'envoi ou qui n'ont pas pu partir restent à la fin.
pub fn merge(shown: &[Message], latest: Vec<Message>) -> Vec<Message> {
    let local = |m: &Message| m.failed || m.id.starts_with(LOCAL_ID);
    let known = |m: &Message| latest.iter().any(|l| l.id == m.id);
    let older = shown.iter().filter(|m| !local(m) && !known(m));
    let failed = shown.iter().filter(|m| local(m));
    older
        .cloned()
        .chain(latest.iter().cloned())
        .chain(failed.cloned())
        .collect()
}

fn str_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |value, key| value.get(*key))?
        .as_str()
}

/// Heure d'un message dans la liste : l'heure aujourd'hui, « hier », puis
/// la date.
fn list_time(then: DateTime<chrono::FixedOffset>, now: DateTime<Utc>) -> String {
    let local = then.with_timezone(&Local);
    let today = now.with_timezone(&Local).date_naive();
    let day = local.date_naive();
    if day == today {
        format!("{:02}:{:02}", local.hour(), local.minute())
    } else if today.pred_opt() == Some(day) {
        gettext("hier")
    } else {
        api::day_month(local.day(), local.month())
    }
}

fn day_label(then: DateTime<chrono::FixedOffset>, now: DateTime<Utc>) -> String {
    let today = now.with_timezone(&Local).date_naive();
    let day = then.with_timezone(&Local).date_naive();
    if day == today {
        gettext("Aujourd'hui")
    } else if today.pred_opt() == Some(day) {
        gettext("Hier")
    } else {
        api::day_month(day.day(), day.month())
    }
}

/// Conversation d'après sa vue JSON ; `me` : mon DID.
pub fn convo_from_json(convo: &Value, me: &str, now: DateTime<Utc>) -> Option<Convo> {
    let id = convo.get("id")?.as_str()?.to_owned();
    let others: Vec<&Value> = convo
        .get("members")?
        .as_array()?
        .iter()
        .filter(|member| member.get("did").and_then(Value::as_str) != Some(me))
        .collect();
    let name = |member: &Value| {
        name_or_handle(
            member.get("displayName").and_then(Value::as_str),
            member
                .get("handle")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
    };
    let group = str_at(convo, &["kind", "name"]).filter(|n| !n.trim().is_empty());
    let title = match group {
        Some(group) => group.to_owned(),
        None if others.is_empty() => gettext("Vous"),
        None => others
            .iter()
            .map(|m| name(m))
            .collect::<Vec<_>>()
            .join(", "),
    };
    let last = convo.get("lastMessage");
    let last_text = match last.and_then(|l| l.get("$type")).and_then(Value::as_str) {
        Some("chat.bsky.convo.defs#deletedMessageView") => gettext("Message supprimé"),
        _ => last
            .and_then(|l| l.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .replace('\n', " "),
    };
    let last_time = last
        .and_then(|l| l.get("sentAt"))
        .and_then(Value::as_str)
        .and_then(parse_datetime)
        .map(|t| list_time(t, now))
        .unwrap_or_default();
    let single = (others.len() == 1 && group.is_none()).then(|| others[0]);
    Some(Convo {
        id,
        title,
        handle: single.and_then(|m| m.get("handle")?.as_str().map(str::to_owned)),
        avatar: single.and_then(|m| m.get("avatar")?.as_str().map(str::to_owned)),
        last_text,
        last_time,
        unread: convo
            .get("unreadCount")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        request: convo.get("status").and_then(Value::as_str) == Some("request"),
    })
}

/// Message d'après sa vue JSON ; les messages supprimés gardent leur place.
pub fn message_from_json(message: &Value, me: &str, now: DateTime<Utc>) -> Option<Message> {
    let deleted = message.get("$type").and_then(Value::as_str)
        == Some("chat.bsky.convo.defs#deletedMessageView");
    let system = message.get("$type").and_then(Value::as_str)
        == Some("chat.bsky.convo.defs#systemMessageView");
    if system {
        return None;
    }
    let sender = str_at(message, &["sender", "did"]).unwrap_or_default();
    let sent = message
        .get("sentAt")
        .and_then(Value::as_str)
        .and_then(parse_datetime);
    Some(Message {
        id: message.get("id")?.as_str()?.to_owned(),
        text: if deleted {
            gettext("Message supprimé")
        } else {
            message
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        },
        mine: sender == me,
        sender: sender.to_owned(),
        time: sent
            .map(|t| {
                let local = t.with_timezone(&Local);
                format!("{:02}:{:02}", local.hour(), local.minute())
            })
            .unwrap_or_default(),
        day: sent.map(|t| day_label(t, now)).unwrap_or_default(),
        failed: false,
    })
}

fn chat_did() -> Did {
    Did::new(CHAT_SERVICE.to_owned()).expect("DID du service de chat")
}

fn to_json(value: &impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap_or_default()
}

/// Conversations acceptées et demandes de message.
pub async fn convos(session: Arc<Session>) -> Result<(Vec<Convo>, Vec<Convo>), ApiError> {
    use atrium_api::chat::bsky::convo::{list_convo_requests, list_convos};
    let api = session.api().await?;
    let chat = api.api_with_proxy(chat_did(), CHAT_SERVICE_TYPE);
    let me = session.account.did.clone();
    let now = Utc::now();
    let accepted = chat
        .chat
        .bsky
        .convo
        .list_convos(
            list_convos::ParametersData {
                cursor: None,
                kind: None,
                limit: 50.try_into().ok(),
                lock_status: None,
                read_state: None,
                status: Some("accepted".to_owned()),
            }
            .into(),
        )
        .await
        .map_err(chat_failed)?;
    let requests = chat
        .chat
        .bsky
        .convo
        .list_convo_requests(
            list_convo_requests::ParametersData {
                cursor: None,
                limit: 50.try_into().ok(),
            }
            .into(),
        )
        .await
        .map_err(chat_failed)?;
    let read = |items: Value| -> Vec<Convo> {
        items
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| convo_from_json(c, &me, now))
            .collect()
    };
    let mut requests = read(to_json(&requests.data.requests));
    for convo in &mut requests {
        convo.request = true;
    }
    let accepted = read(to_json(&accepted.data.convos))
        .into_iter()
        .filter(|c| !c.request)
        .collect();
    Ok((accepted, requests))
}

/// Messages d'une conversation, du plus ancien au plus récent : les
/// derniers, ou ceux d'avant `cursor`.
pub async fn messages(
    session: Arc<Session>,
    convo_id: String,
    cursor: Option<String>,
) -> Result<MessagePage, ApiError> {
    use atrium_api::chat::bsky::convo::get_messages;
    let api = session.api().await?;
    let chat = api.api_with_proxy(chat_did(), CHAT_SERVICE_TYPE);
    let output = chat
        .chat
        .bsky
        .convo
        .get_messages(
            get_messages::ParametersData {
                convo_id,
                cursor,
                limit: 50.try_into().ok(),
            }
            .into(),
        )
        .await
        .map_err(chat_failed)?;
    let cursor = output.data.cursor.clone();
    let me = session.account.did.clone();
    let now = Utc::now();
    let mut messages: Vec<Message> = to_json(&output.data.messages)
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| message_from_json(m, &me, now))
        .collect();
    // L'API donne les plus récents d'abord.
    messages.reverse();
    Ok((messages, cursor))
}

pub async fn send(
    session: Arc<Session>,
    convo_id: String,
    text: String,
) -> Result<Message, ApiError> {
    use atrium_api::chat::bsky::convo::{defs, send_message};
    let api = session.api().await?;
    let chat = api.api_with_proxy(chat_did(), CHAT_SERVICE_TYPE);
    let output = chat
        .chat
        .bsky
        .convo
        .send_message(
            send_message::InputData {
                convo_id,
                message: defs::MessageInputData {
                    embed: None,
                    facets: None,
                    reply_to: None,
                    text,
                }
                .into(),
            }
            .into(),
        )
        .await
        .map_err(chat_failed)?;
    message_from_json(&to_json(&output.data), &session.account.did, Utc::now())
        .ok_or_else(|| ApiError::new(gettext("Réponse illisible du service de messages.")))
}

/// Conversation avec un compte (créée au besoin, `getConvoForMembers`).
pub async fn convo_for(session: Arc<Session>, did: String) -> Result<Convo, ApiError> {
    use atrium_api::chat::bsky::convo::get_convo_for_members;
    let api = session.api().await?;
    let chat = api.api_with_proxy(chat_did(), CHAT_SERVICE_TYPE);
    let member = Did::new(did).map_err(|_| api::invalid_actor())?;
    let output = chat
        .chat
        .bsky
        .convo
        .get_convo_for_members(
            get_convo_for_members::ParametersData {
                members: vec![member],
            }
            .into(),
        )
        .await
        .map_err(chat_failed)?;
    convo_from_json(
        &to_json(&output.data.convo),
        &session.account.did,
        Utc::now(),
    )
    .ok_or_else(|| ApiError::new(gettext("Réponse illisible du service de messages.")))
}

/// Marque la conversation comme lue.
pub async fn mark_read(session: Arc<Session>, convo_id: String) -> Result<(), ApiError> {
    use atrium_api::chat::bsky::convo::update_read;
    let api = session.api().await?;
    let chat = api.api_with_proxy(chat_did(), CHAT_SERVICE_TYPE);
    chat.chat
        .bsky
        .convo
        .update_read(
            update_read::InputData {
                convo_id,
                message_id: None,
            }
            .into(),
        )
        .await
        .map(|_| ())
        .map_err(chat_failed)
}

/// Accepte une demande de message.
pub async fn accept(session: Arc<Session>, convo_id: String) -> Result<(), ApiError> {
    use atrium_api::chat::bsky::convo::accept_convo;
    let api = session.api().await?;
    let chat = api.api_with_proxy(chat_did(), CHAT_SERVICE_TYPE);
    chat.chat
        .bsky
        .convo
        .accept_convo(accept_convo::InputData { convo_id }.into())
        .await
        .map(|_| ())
        .map_err(chat_failed)
}

/// Erreur du service de messages. Une session ouverte avant que Tsubame
/// demande l'accès aux messages n'a pas la portée nécessaire : le dire.
fn chat_failed(error: impl std::fmt::Display) -> ApiError {
    let text = error.to_string();
    if text.contains("scope") || text.contains("Bad token method") {
        return ApiError::new(gettext(
            "La messagerie n'est pas autorisée pour cette session. Déconnectez-vous puis reconnectez-vous pour l'autoriser.",
        ));
    }
    request_failed(text)
}

/// Conversations et messages de démonstration, repris de la maquette
/// `Messages.dc.html`.
pub fn demo_convos() -> (Vec<Convo>, Vec<Convo>) {
    let convo =
        |id: &str, title: &str, handle: Option<&str>, last: &str, time: &str, unread| Convo {
            id: id.to_owned(),
            title: title.to_owned(),
            handle: handle.map(str::to_owned),
            avatar: None,
            last_text: last.to_owned(),
            last_time: time.to_owned(),
            unread,
            request: false,
        };
    let accepted = vec![
        convo(
            "lucie",
            "Lucie Morel",
            Some("lucie.example.social"),
            "Je t'envoie le lien du dépôt tout de suite",
            "14:40",
            0,
        ),
        convo(
            "nadia",
            "Nadia Safi",
            Some("nadia.example.social"),
            "On se voit à la réunion de jeudi ?",
            "13:02",
            1,
        ),
        convo(
            "romain",
            "Romain Bailly",
            Some("romain.example.social"),
            "Merci pour la capture !",
            "hier",
            0,
        ),
        convo(
            "groupe",
            "Traduction fr",
            None,
            "Aya : le glossaire est à jour",
            "18 nov.",
            0,
        ),
    ];
    let mut request = convo(
        "hugo",
        "Hugo Renard",
        Some("hugo.example.social"),
        "Bonjour, une question sur la version 1.8",
        "11:15",
        1,
    );
    request.request = true;
    (accepted, vec![request])
}

pub fn demo_messages(convo_id: &str) -> Vec<Message> {
    let message = |id: usize, text: &str, mine: bool, time: &str| Message {
        id: id.to_string(),
        text: text.to_owned(),
        mine,
        sender: String::new(),
        time: time.to_owned(),
        day: gettext("Aujourd'hui"),
        failed: false,
    };
    match convo_id {
        "lucie" => vec![
            message(
                1,
                "Salut, tu as vu les nouveautés de la bêta ?",
                false,
                "14:31",
            ),
            message(
                2,
                "Oui, le sélecteur de fichiers est bien mieux. J'ai deux ou trois retours sur l'accessibilité.",
                true,
                "14:33",
            ),
            message(3, "Parfait, note-les dans le dépôt.", false, "14:35"),
            message(4, "Merci, je regarde ça cet après-midi.", true, "14:38"),
            message(
                5,
                "Je t'envoie le lien du dépôt tout de suite",
                false,
                "14:40",
            ),
        ],
        _ => demo_convos()
            .0
            .into_iter()
            .chain(demo_convos().1)
            .filter(|c| c.id == convo_id)
            .map(|c| message(1, &c.last_text, false, &c.last_time))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-20T15:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn conversations_are_read() {
        let convo = serde_json::json!({
            "id": "c1",
            "rev": "1",
            "members": [
                {"did": "did:plc:moi", "handle": "vous.bsky.social"},
                {"did": "did:plc:lucie", "handle": "lucie.example.social", "displayName": "Lucie Morel",
                 "avatar": "https://cdn.example/lucie.jpg"}
            ],
            "lastMessage": {
                "$type": "chat.bsky.convo.defs#messageView",
                "id": "m9", "rev": "9", "text": "Je t'envoie\nle lien",
                "sender": {"did": "did:plc:lucie"},
                "sentAt": "2026-11-20T12:00:00.000Z"
            },
            "muted": false,
            "unreadCount": 2,
            "status": "accepted"
        });
        let convo = convo_from_json(&convo, "did:plc:moi", now()).unwrap();
        assert_eq!(convo.title, "Lucie Morel");
        assert_eq!(convo.handle.as_deref(), Some("lucie.example.social"));
        assert_eq!(convo.last_text, "Je t'envoie le lien");
        assert_eq!(convo.unread, 2);
        assert!(!convo.request);
        // Heure locale, aujourd'hui : « HH:MM ».
        assert_eq!(convo.last_time.len(), 5);
    }

    #[test]
    fn groups_and_requests() {
        let convo = serde_json::json!({
            "id": "g1",
            "members": [
                {"did": "did:plc:moi", "handle": "vous.bsky.social"},
                {"did": "did:plc:a", "handle": "a.example"},
                {"did": "did:plc:b", "handle": "b.example"}
            ],
            "kind": {"$type": "chat.bsky.convo.defs#groupConvo", "name": "Traduction fr"},
            "lastMessage": {"$type": "chat.bsky.convo.defs#deletedMessageView", "id": "x",
                            "sender": {"did": "did:plc:a"}, "sentAt": "2026-11-18T09:00:00.000Z"},
            "unreadCount": 0,
            "status": "request"
        });
        let convo = convo_from_json(&convo, "did:plc:moi", now()).unwrap();
        assert_eq!(convo.title, "Traduction fr");
        assert_eq!(convo.handle, None);
        assert_eq!(convo.last_text, "Message supprimé");
        assert!(convo.request);
    }

    #[test]
    fn messages_know_who_wrote_them() {
        let mine = serde_json::json!({
            "$type": "chat.bsky.convo.defs#messageView",
            "id": "m1", "rev": "1", "text": "Merci !",
            "sender": {"did": "did:plc:moi"},
            "sentAt": "2026-11-19T10:00:00.000Z"
        });
        let message = message_from_json(&mine, "did:plc:moi", now()).unwrap();
        assert!(message.mine);
        assert_eq!(message.text, "Merci !");
        assert_eq!(message.day, "Hier");
        let system =
            serde_json::json!({"$type": "chat.bsky.convo.defs#systemMessageView", "id": "s"});
        assert_eq!(message_from_json(&system, "did:plc:moi", now()), None);
    }

    #[test]
    fn refreshed_messages_are_merged() {
        let message = |id: &str, failed| Message {
            id: id.into(),
            text: id.into(),
            mine: true,
            sender: String::new(),
            time: String::new(),
            day: String::new(),
            failed,
        };
        let shown = vec![
            message("1", false),
            message("2", false),
            message("local:1", true),
            message("3", false),
        ];
        let latest = vec![
            message("2", false),
            message("3", false),
            message("4", false),
        ];
        let ids: Vec<String> = merge(&shown, latest)
            .into_iter()
            .map(|m| if m.failed { "échec".into() } else { m.id })
            .collect();
        assert_eq!(ids, ["1", "2", "3", "4", "échec"]);
    }
}
