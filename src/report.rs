//! Signaler un post ou un compte au service de modération de Bluesky
//! (`com.atproto.moderation.createReport`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use atrium_api::com::atproto::moderation::{create_report, defs};
use atrium_api::types::Union;
use atrium_api::types::string::Did;
use gettextrs::gettext;
use gtk::glib;

use crate::api::{self, ApiError, request_failed};
use crate::auth::Session;
use crate::runtime::runtime;

/// Service de modération de Bluesky, à qui vont les signalements.
const MODERATION_SERVICE: &str = "did:plc:ar7c4by46qjdydhdevvrndac";
const MODERATION_SERVICE_TYPE: &str = "atproto_labeler";
/// Longueur maximale des précisions acceptée par Bluesky.
pub const MAX_DETAILS: usize = 2000;

/// Ce qui est signalé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    Post {
        uri: String,
        cid: String,
    },
    /// Compte, par son identifiant ou son DID.
    Account(String),
    /// Message d'une conversation, avec le DID de son auteur.
    Message {
        did: String,
        convo_id: String,
        message_id: String,
    },
}

/// Motifs proposés, dans l'ordre affiché.
pub fn reasons() -> [(&'static str, String, String); 6] {
    [
        (
            defs::REASON_SPAM,
            gettext("Spam"),
            gettext("Messages répétés, publicité, faux engagement"),
        ),
        (
            defs::REASON_MISLEADING,
            gettext("Contenu trompeur"),
            gettext("Usurpation d'identité, fausses informations"),
        ),
        (
            defs::REASON_SEXUAL,
            gettext("Contenu sexuel non étiqueté"),
            gettext("Nudité ou contenu pour adultes sans étiquette"),
        ),
        (
            defs::REASON_RUDE,
            gettext("Comportement hostile"),
            gettext("Harcèlement, insultes, intimidation"),
        ),
        (
            defs::REASON_VIOLATION,
            gettext("Infraction aux règles"),
            gettext("Contenu illégal ou contraire aux conditions d'utilisation"),
        ),
        (
            defs::REASON_OTHER,
            gettext("Autre"),
            gettext("Un problème qui n'est pas dans cette liste"),
        ),
    ]
}

/// Corps de la requête : motif, précisions (vides : absentes) et sujet.
pub fn input(
    subject: &Subject,
    did: Option<Did>,
    reason: &str,
    details: &str,
) -> Result<create_report::Input, ApiError> {
    use atrium_api::com::atproto::admin::defs::RepoRefData;
    use atrium_api::com::atproto::repo::strong_ref;
    let invalid = || ApiError::new(gettext("Ce contenu ne peut pas être signalé."));
    let subject = match subject {
        // Pas de variante pour les messages dans l'union : elle est ouverte,
        // on la lit depuis le JSON.
        Subject::Message {
            did,
            convo_id,
            message_id,
        } => {
            let subject = serde_json::from_value(serde_json::json!({
                "$type": "chat.bsky.convo.defs#messageRef",
                "did": did,
                "convoId": convo_id,
                "messageId": message_id,
            }))
            .map_err(|_| invalid())?;
            let details: String = details.trim().chars().take(MAX_DETAILS).collect();
            return Ok(create_report::InputData {
                mod_tool: None,
                reason: (!details.is_empty()).then_some(details),
                reason_type: reason.to_owned(),
                subject,
            }
            .into());
        }
        Subject::Post { uri, cid } => {
            create_report::InputSubjectRefs::ComAtprotoRepoStrongRefMain(Box::new(
                strong_ref::MainData {
                    cid: cid.parse().map_err(|_| invalid())?,
                    uri: uri.clone(),
                }
                .into(),
            ))
        }
        Subject::Account(_) => {
            create_report::InputSubjectRefs::ComAtprotoAdminDefsRepoRef(Box::new(
                RepoRefData {
                    did: did.ok_or_else(invalid)?,
                }
                .into(),
            ))
        }
    };
    let details: String = details.trim().chars().take(MAX_DETAILS).collect();
    Ok(create_report::InputData {
        mod_tool: None,
        reason: (!details.is_empty()).then_some(details),
        reason_type: reason.to_owned(),
        subject: Union::Refs(subject),
    }
    .into())
}

/// Envoie le signalement au service `service` (DID) : celui de Bluesky, ou
/// un service d'étiquetage suivi.
pub async fn send(
    session: Arc<Session>,
    subject: Subject,
    reason: String,
    details: String,
    service: String,
) -> Result<(), ApiError> {
    let did = match &subject {
        Subject::Account(actor) if actor.starts_with("did:") => Some(actor.clone()),
        Subject::Account(actor) => Some(
            Box::pin(api::profile(session.clone(), actor.clone()))
                .await?
                .did,
        ),
        Subject::Post { .. } | Subject::Message { .. } => None,
    }
    .map(|did| Did::new(did).map_err(|_| ApiError::new(gettext("Ce compte n'existe pas."))))
    .transpose()?;
    let input = input(&subject, did, &reason, &details)?;
    let api = session.api().await?;
    let service = Did::new(service)
        .or_else(|_| Did::new(MODERATION_SERVICE.to_owned()))
        .expect("DID du service de modération");
    let proxied = api.api_with_proxy(service, MODERATION_SERVICE_TYPE);
    Box::pin(proxied.com.atproto.moderation.create_report(input))
        .await
        .map_err(request_failed)?;
    Ok(())
}

/// Boîte « Signaler » : un motif, des précisions facultatives, Envoyer.
/// `on_sent` reçoit le résultat ; `None` en démonstration.
pub fn present(
    parent: &impl IsA<gtk::Widget>,
    session: Option<Arc<Session>>,
    subject: Subject,
    on_sent: impl Fn(Result<(), ApiError>) + 'static,
) {
    let title = match &subject {
        Subject::Post { .. } => gettext("Signaler ce post"),
        Subject::Account(handle) => gettext("Signaler @{}").replace("{}", handle),
        Subject::Message { .. } => gettext("Signaler ce message"),
    };
    let cancel = gtk::Button::with_label(&gettext("Annuler"));
    let send_button = gtk::Button::builder()
        .label(gettext("Envoyer"))
        .css_classes(["suggested-action"])
        .sensitive(false)
        .build();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&send_button);

    let page = adw::PreferencesPage::new();
    let reasons_group = adw::PreferencesGroup::builder()
        .title(gettext("Motif"))
        .description(gettext(
            "Le signalement est envoyé au service de modération de Bluesky. La personne signalée n'en est pas informée.",
        ))
        .build();
    let chosen: Rc<RefCell<Option<&'static str>>> = Rc::default();
    let mut first: Option<gtk::CheckButton> = None;
    for (reason, title, subtitle) in reasons() {
        let check = gtk::CheckButton::builder()
            .valign(gtk::Align::Center)
            .build();
        if let Some(first) = &first {
            check.set_group(Some(first));
        } else {
            first = Some(check.clone());
        }
        let row = adw::ActionRow::builder()
            .title(&title)
            .subtitle(&subtitle)
            .activatable_widget(&check)
            .build();
        row.add_prefix(&check);
        let (chosen, send_button) = (chosen.clone(), send_button.clone());
        check.connect_toggled(move |check| {
            if check.is_active() {
                chosen.replace(Some(reason));
                send_button.set_sensitive(true);
            }
        });
        reasons_group.add(&row);
    }
    page.add(&reasons_group);
    // Destinataire : Bluesky, ou un service d'étiquetage suivi.
    let service: Rc<RefCell<String>> = Rc::new(RefCell::new(MODERATION_SERVICE.to_owned()));
    let labelers = crate::moderation::labeler_names();
    if !labelers.is_empty() {
        let services = adw::PreferencesGroup::builder()
            .title(gettext("Destinataire"))
            .build();
        let mut first: Option<gtk::CheckButton> = None;
        for (did, name) in
            std::iter::once((MODERATION_SERVICE.to_owned(), gettext("Modération Bluesky")))
                .chain(labelers)
        {
            let check = gtk::CheckButton::builder()
                .valign(gtk::Align::Center)
                .active(first.is_none())
                .build();
            if let Some(first) = &first {
                check.set_group(Some(first));
            } else {
                first = Some(check.clone());
            }
            let row = adw::ActionRow::builder()
                .title(&name)
                .use_markup(false)
                .activatable_widget(&check)
                .build();
            row.add_prefix(&check);
            let service = service.clone();
            check.connect_toggled(move |check| {
                if check.is_active() {
                    service.replace(did.clone());
                }
            });
            services.add(&row);
        }
        page.add(&services);
    }
    let details_group = adw::PreferencesGroup::new();
    let details = adw::EntryRow::builder()
        .title(gettext("Précisions (facultatif)"))
        .build();
    details_group.add(&details);
    page.add(&details_group);

    let toolbar = adw::ToolbarView::builder().content(&page).build();
    toolbar.add_top_bar(&header);
    let dialog = adw::Dialog::builder()
        .title(&title)
        .content_width(460)
        .content_height(620)
        .child(&toolbar)
        .build();
    let weak = dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(dialog) = weak.upgrade() {
            dialog.close();
        }
    });
    let on_sent = Rc::new(on_sent);
    let weak = dialog.downgrade();
    send_button.connect_clicked(move |button| {
        let Some(reason) = *chosen.borrow() else {
            return;
        };
        let Some(dialog) = weak.upgrade() else { return };
        let Some(session) = session.clone() else {
            dialog.close();
            on_sent(Ok(()));
            return;
        };
        button.set_sensitive(false);
        let task = runtime().spawn(send(
            session,
            subject.clone(),
            reason.to_owned(),
            details.text().to_string(),
            service.borrow().clone(),
        ));
        let (on_sent, button) = (on_sent.clone(), button.clone());
        glib::spawn_future_local(async move {
            let result = task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))));
            if result.is_ok() {
                dialog.close();
            } else {
                button.set_sensitive(true);
            }
            on_sent(result);
        });
    });
    dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_name_their_subject() {
        let post = Subject::Post {
            uri: "at://did:plc:r/app.bsky.feed.post/1".into(),
            cid: "bafyreie5737gdxlw5i64vzichcalba3z2v5n6icifvx5xytvske7mr3hpm".into(),
        };
        let json =
            serde_json::to_value(input(&post, None, defs::REASON_SPAM, "  ").unwrap()).unwrap();
        assert_eq!(json["reasonType"], "com.atproto.moderation.defs#reasonSpam");
        assert_eq!(json["subject"]["$type"], "com.atproto.repo.strongRef");
        assert_eq!(
            json["subject"]["uri"],
            "at://did:plc:r/app.bsky.feed.post/1"
        );
        assert!(json.get("reason").is_none());

        let account = Subject::Account("romain.example.social".into());
        let did = Did::new("did:plc:r".into()).ok();
        let long = "x".repeat(MAX_DETAILS + 10);
        let json =
            serde_json::to_value(input(&account, did, defs::REASON_RUDE, &long).unwrap()).unwrap();
        assert_eq!(json["subject"]["$type"], "com.atproto.admin.defs#repoRef");
        assert_eq!(json["subject"]["did"], "did:plc:r");
        assert_eq!(json["reason"].as_str().unwrap().len(), MAX_DETAILS);
        assert!(input(&account, None, defs::REASON_RUDE, "").is_err());

        let message = Subject::Message {
            did: "did:plc:r".into(),
            convo_id: "c1".into(),
            message_id: "m1".into(),
        };
        let json =
            serde_json::to_value(input(&message, None, defs::REASON_SPAM, "").unwrap()).unwrap();
        assert_eq!(json["subject"]["$type"], "chat.bsky.convo.defs#messageRef");
        assert_eq!(json["subject"]["messageId"], "m1");
    }
}
