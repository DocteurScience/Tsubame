//! Listes : créer, renommer, supprimer, gérer les membres ; s'abonner aux
//! listes de modération des autres (masquer ou bloquer leurs comptes).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use atrium_api::types::string::{Nsid, RecordKey};
use gettextrs::gettext;
use gtk::{gdk, gio, glib};
use serde_json::{Value, json};

use crate::api::{self, ApiError, invalid_actor, request_failed};
use crate::auth::Session;
use crate::data::{self, Actor, List};
use crate::edit_profile::ImageChange;
use crate::explore::actor_row;
use crate::photo::{self, Prepared};
use crate::profile::Host;
use crate::runtime::runtime;

const LIST: &str = "app.bsky.graph.list";
const LIST_ITEM: &str = "app.bsky.graph.listitem";
const LIST_BLOCK: &str = "app.bsky.graph.listblock";
/// Longueurs maximales du nom et de la description d'une liste.
const MAX_NAME: usize = 64;
const MAX_DESCRIPTION: usize = 300;

/// Enregistrement d'une nouvelle liste.
pub fn list_record(name: &str, description: &str, moderation: bool, created_at: &str) -> Value {
    let mut record = json!({
        "$type": LIST,
        "purpose": if moderation {
            "app.bsky.graph.defs#modlist"
        } else {
            "app.bsky.graph.defs#curatelist"
        },
        "name": name,
        "createdAt": created_at,
    });
    if !description.is_empty() {
        record["description"] = json!(description);
    }
    record
}

/// Crée un enregistrement dans mon dépôt ; rend son adresse.
async fn create_record(
    session: &Session,
    collection: &str,
    record: Value,
) -> Result<String, ApiError> {
    use atrium_api::com::atproto::repo::create_record;
    let api = session.api().await?;
    let output = api
        .api
        .com
        .atproto
        .repo
        .create_record(
            create_record::InputData {
                collection: Nsid::new(collection.to_owned()).map_err(|_| invalid_actor())?,
                record: serde_json::from_value(record).map_err(|_| invalid_actor())?,
                repo: session.account.did.parse().map_err(|_| invalid_actor())?,
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

fn now() -> String {
    atrium_api::types::string::Datetime::now()
        .as_str()
        .to_owned()
}

/// Envoie une image préparée ; rend son blob, à placer dans un enregistrement.
async fn upload_image(session: &Session, image: &Prepared) -> Result<Value, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .com
        .atproto
        .repo
        .upload_blob(image.bytes.clone())
        .await
        .map_err(|e| {
            ApiError::new(
                gettext("Une image n'a pas pu être envoyée : {}").replace("{}", &e.to_string()),
            )
        })?;
    serde_json::to_value(&output.data.blob).map_err(|_| invalid_actor())
}

/// Applique le changement d'image à l'enregistrement d'une liste.
pub fn apply_avatar(record: &mut Value, avatar: &ImageChange, blob: Option<Value>) {
    let Some(map) = record.as_object_mut() else {
        return;
    };
    match (avatar, blob) {
        (ImageChange::Keep, _) => {}
        (ImageChange::Replace(_), Some(blob)) => {
            map.insert("avatar".into(), blob);
        }
        _ => {
            map.remove("avatar");
        }
    }
}

async fn avatar_blob(session: &Session, avatar: &ImageChange) -> Result<Option<Value>, ApiError> {
    match avatar {
        ImageChange::Replace(image) => Ok(Some(Box::pin(upload_image(session, image)).await?)),
        _ => Ok(None),
    }
}

pub async fn create_list(
    session: Arc<Session>,
    name: String,
    description: String,
    moderation: bool,
    avatar: ImageChange,
) -> Result<String, ApiError> {
    let mut record = list_record(&name, &description, moderation, &now());
    let blob = Box::pin(avatar_blob(&session, &avatar)).await?;
    apply_avatar(&mut record, &avatar, blob);
    create_record(&session, LIST, record).await
}

/// Renomme une liste (ou un starter pack), change sa description et son image, en gardant le
/// reste de son enregistrement, s'il n'a pas changé entre-temps.
pub async fn update_list(
    session: Arc<Session>,
    uri: String,
    name: String,
    description: String,
    avatar: ImageChange,
) -> Result<(), ApiError> {
    let blob = Box::pin(avatar_blob(&session, &avatar)).await?;
    Box::pin(edit_record(session, uri, move |record| {
        record["name"] = json!(name);
        match description.is_empty() {
            true => {
                if let Some(map) = record.as_object_mut() {
                    map.remove("description");
                    map.remove("descriptionFacets");
                }
            }
            false => record["description"] = json!(description),
        }
        apply_avatar(record, &avatar, blob);
    }))
    .await
}

/// Relit un enregistrement de mon dépôt, le modifie avec `edit` et le réécrit, s'il n'a pas
/// changé entre-temps (`swapRecord`).
async fn edit_record(
    session: Arc<Session>,
    uri: String,
    edit: impl FnOnce(&mut Value) + Send,
) -> Result<(), ApiError> {
    use atrium_api::com::atproto::repo::{get_record, put_record};
    // La collection vient de l'adresse.
    let (repo, collection, rkey) = api::parse_record_uri(&uri).ok_or_else(invalid_actor)?;
    let rkey = RecordKey::new(rkey.to_owned()).map_err(|_| invalid_actor())?;
    let api = session.api().await?;
    let current = api
        .api
        .com
        .atproto
        .repo
        .get_record(
            get_record::ParametersData {
                cid: None,
                collection: Nsid::new(collection.to_owned()).map_err(|_| invalid_actor())?,
                repo: repo.parse().map_err(|_| invalid_actor())?,
                rkey: rkey.clone(),
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    let mut record = serde_json::to_value(&current.data.value).unwrap_or_default();
    edit(&mut record);
    api.api
        .com
        .atproto
        .repo
        .put_record(
            put_record::InputData {
                collection: Nsid::new(collection.to_owned()).map_err(|_| invalid_actor())?,
                record: serde_json::from_value(record).map_err(|_| invalid_actor())?,
                repo: repo.parse().map_err(|_| invalid_actor())?,
                rkey,
                swap_commit: None,
                swap_record: current.data.cid,
                validate: None,
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(())
}

/// Nombre maximal de flux dans un starter pack.
pub const MAX_PACK_FEEDS: usize = 3;

/// Place les flux `feeds` dans l'enregistrement d'un starter pack (aucun : retire le champ).
pub fn set_feeds(record: &mut Value, feeds: &[String]) {
    let Some(map) = record.as_object_mut() else {
        return;
    };
    if feeds.is_empty() {
        map.remove("feeds");
    } else {
        let items: Vec<Value> = feeds
            .iter()
            .take(MAX_PACK_FEEDS)
            .map(|uri| json!({ "uri": uri }))
            .collect();
        map.insert("feeds".into(), Value::Array(items));
    }
}

/// Remplace les flux d'un starter pack.
pub async fn update_pack_feeds(
    session: Arc<Session>,
    uri: String,
    feeds: Vec<String>,
) -> Result<(), ApiError> {
    Box::pin(edit_record(session, uri, move |record| {
        set_feeds(record, &feeds)
    }))
    .await
}

const STARTER_PACK: &str = "app.bsky.graph.starterpack";

/// Enregistrement d'un starter pack, qui renvoie à sa liste de comptes.
pub fn starter_pack_record(name: &str, description: &str, list: &str, created_at: &str) -> Value {
    let mut record = json!({
        "$type": STARTER_PACK,
        "name": name,
        "list": list,
        "createdAt": created_at,
    });
    if !description.is_empty() {
        record["description"] = json!(description);
    }
    record
}

/// Crée un starter pack : sa liste de comptes (« liste de référence », qui
/// n'apparaît pas parmi les listes), puis l'enregistrement qui la cite.
/// Rend l'adresse du starter pack.
pub async fn create_starter_pack(
    session: Arc<Session>,
    name: String,
    description: String,
) -> Result<String, ApiError> {
    let created_at = now();
    let mut list = list_record(&name, "", false, &created_at);
    list["purpose"] = json!("app.bsky.graph.defs#referencelist");
    let list = create_record(&session, LIST, list).await?;
    let record = starter_pack_record(&name, &description, &list, &created_at);
    create_record(&session, STARTER_PACK, record).await
}

/// Supprime un starter pack et sa liste de comptes.
pub async fn delete_starter_pack(
    session: Arc<Session>,
    uri: String,
    list: Option<String>,
) -> Result<(), ApiError> {
    api::delete(session.clone(), uri).await?;
    if let Some(list) = list {
        api::delete(session, list).await?;
    }
    Ok(())
}

/// Boîte de recherche d'un compte ; `on_pick` reçoit celui choisi.
pub fn pick_account(
    parent: &impl IsA<gtk::Widget>,
    title: &str,
    session: Option<Arc<Session>>,
    on_pick: impl Fn(Actor) + 'static,
) {
    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(420)
        .content_height(480)
        .build();
    let entry = gtk::SearchEntry::builder()
        .placeholder_text(gettext("Rechercher un compte"))
        .search_delay(300)
        .build();
    entry.update_property(&[gtk::accessible::Property::Label(&gettext(
        "Rechercher un compte",
    ))]);
    let results = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .valign(gtk::Align::Start)
        .visible(false)
        .css_classes(["boxed-list"])
        .build();
    results.update_property(&[gtk::accessible::Property::Label(&gettext("Comptes"))]);
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&entry);
    content.append(&results);
    let view = adw::ToolbarView::builder()
        .content(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&content)
                .build(),
        )
        .build();
    view.add_top_bar(&adw::HeaderBar::new());
    dialog.set_child(Some(&view));

    let found: Rc<std::cell::RefCell<Vec<Actor>>> = Rc::default();
    let show = {
        let (results, found) = (results.clone(), found.clone());
        Rc::new(move |actors: Vec<Actor>| {
            results.remove_all();
            for actor in &actors {
                results.append(
                    &adw::ActionRow::builder()
                        .title(&actor.name)
                        .subtitle(format!("@{}", actor.handle))
                        .use_markup(false)
                        .activatable(true)
                        .build(),
                );
            }
            results.set_visible(!actors.is_empty());
            found.replace(actors);
        })
    };
    entry.connect_search_changed(move |entry| {
        let query = entry.text().trim().to_owned();
        if query.is_empty() {
            show(Vec::new());
            return;
        }
        let Some(session) = session.clone() else {
            let lower = query.to_lowercase();
            show(
                data::demo_actors()
                    .into_iter()
                    .filter(|a| a.name.to_lowercase().contains(&lower) || a.handle.contains(&lower))
                    .collect(),
            );
            return;
        };
        let task = runtime().spawn(api::typeahead(session, query.clone()));
        let (show, entry) = (show.clone(), entry.downgrade());
        glib::spawn_future_local(async move {
            let Ok(Ok(actors)) = task.await else { return };
            if entry.upgrade().map(|e| e.text().trim() == query) == Some(true) {
                show(actors);
            }
        });
    });
    let closing = dialog.downgrade();
    results.connect_row_activated(move |_, row| {
        let actor = found.borrow().get(row.index() as usize).cloned();
        if let Some(actor) = actor {
            if let Some(dialog) = closing.upgrade() {
                dialog.close();
            }
            on_pick(actor);
        }
    });
    dialog.present(Some(parent));
    entry.grab_focus();
}

/// Boîte « Nouveau starter pack » ou « Modifier le starter pack » : nom
/// et description. `save` reçoit les deux.
pub fn starter_pack_dialog(
    parent: &impl IsA<gtk::Widget>,
    current: Option<(&str, &str)>,
    save: impl Fn(String, String) + 'static,
) {
    let alert = adw::AlertDialog::new(
        Some(&if current.is_some() {
            gettext("Modifier le starter pack")
        } else {
            gettext("Nouveau starter pack")
        }),
        Some(&gettext(
            "Une sélection de comptes à suivre d'un coup. Ajoutez-y des comptes ensuite.",
        )),
    );
    let name = adw::EntryRow::builder().title(gettext("Nom")).build();
    let description = adw::EntryRow::builder()
        .title(gettext("Description"))
        .build();
    if let Some((n, d)) = current {
        name.set_text(n);
        description.set_text(d);
    }
    let rows = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    rows.append(&name);
    rows.append(&description);
    alert.set_extra_child(Some(&rows));
    alert.add_responses(&[
        ("cancel", &gettext("Annuler")),
        (
            "save",
            &if current.is_some() {
                gettext("Enregistrer")
            } else {
                gettext("Créer")
            },
        ),
    ]);
    alert.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    alert.set_close_response("cancel");
    let valid = |name: &adw::EntryRow| {
        let n = grapheme_count(name.text().trim());
        n > 0 && n <= MAX_NAME
    };
    alert.set_response_enabled("save", valid(&name));
    {
        let alert = alert.downgrade();
        name.connect_changed(move |name| {
            if let Some(alert) = alert.upgrade() {
                alert.set_response_enabled("save", valid(name));
            }
        });
    }
    alert.connect_response(Some("save"), move |_, _| {
        let description: String = description
            .text()
            .trim()
            .chars()
            .take(MAX_DESCRIPTION)
            .collect();
        save(name.text().trim().to_owned(), description);
    });
    alert.present(Some(parent));
}

/// Ajoute un compte à une liste ; rend l'adresse de l'élément.
pub async fn add_member(
    session: Arc<Session>,
    list: String,
    did: String,
) -> Result<String, ApiError> {
    let record = json!({"$type": LIST_ITEM, "subject": did, "list": list, "createdAt": now()});
    create_record(&session, LIST_ITEM, record).await
}

/// Membres d'une liste, avec l'adresse de leur élément (pour les retirer).
pub async fn items(session: Arc<Session>, uri: String) -> Result<Vec<(Actor, String)>, ApiError> {
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
        .map(|item| (api::actor_from_view(&item.subject), item.uri.clone()))
        .collect())
}

/// Mes listes, et pour chacune l'élément qui y place `actor`, s'il y est.
pub async fn memberships(
    session: Arc<Session>,
    actor: String,
) -> Result<Vec<(List, Option<String>)>, ApiError> {
    let api = session.api().await?;
    let output = api
        .api
        .app
        .bsky
        .graph
        .get_lists_with_membership(
            atrium_api::app::bsky::graph::get_lists_with_membership::ParametersData {
                actor: actor.parse().map_err(|_| invalid_actor())?,
                cursor: None,
                limit: 100.try_into().ok(),
                purposes: None,
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    Ok(output
        .data
        .lists_with_membership
        .iter()
        .map(|entry| {
            (
                api::list_from_view(&entry.list),
                entry.list_item.as_ref().map(|item| item.uri.clone()),
            )
        })
        .collect())
}

/// Masque (ou ne masque plus) les comptes d'une liste de modération.
pub async fn set_muted(session: Arc<Session>, uri: String, muted: bool) -> Result<(), ApiError> {
    let api = session.api().await?;
    let graph = &api.api.app.bsky.graph;
    if muted {
        graph
            .mute_actor_list(
                atrium_api::app::bsky::graph::mute_actor_list::InputData { list: uri }.into(),
            )
            .await
            .map_err(request_failed)
    } else {
        graph
            .unmute_actor_list(
                atrium_api::app::bsky::graph::unmute_actor_list::InputData { list: uri }.into(),
            )
            .await
            .map_err(request_failed)
    }
}

/// Bloque les comptes d'une liste de modération ; rend l'adresse du blocage.
pub async fn block(session: Arc<Session>, uri: String) -> Result<String, ApiError> {
    let record = json!({"$type": LIST_BLOCK, "subject": uri, "createdAt": now()});
    create_record(&session, LIST_BLOCK, record).await
}

/// Listes de modération suivies : celles dont je masque ou bloque les
/// comptes (`getListMutes`, `getListBlocks`).
pub async fn subscribed(session: Arc<Session>) -> Result<Vec<List>, ApiError> {
    use atrium_api::app::bsky::graph::{get_list_blocks, get_list_mutes};
    let api = session.api().await?;
    let graph = &api.api.app.bsky.graph;
    let mutes = graph
        .get_list_mutes(
            get_list_mutes::ParametersData {
                cursor: None,
                limit: 100.try_into().ok(),
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    let blocks = graph
        .get_list_blocks(
            get_list_blocks::ParametersData {
                cursor: None,
                limit: 100.try_into().ok(),
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    let mut lists: Vec<List> = Vec::new();
    for list in mutes.data.lists.iter().chain(blocks.data.lists.iter()) {
        let list = api::list_from_view(list);
        if !lists.iter().any(|l| l.uri == list.uri) {
            lists.push(list);
        }
    }
    Ok(lists)
}

fn grapheme_count(text: &str) -> usize {
    crate::richtext::grapheme_count(text)
}

/// Ligne « Image » de la boîte d'une liste : l'image actuelle, « Choisir » et « Retirer ».
/// `change` reçoit le choix.
fn image_row(
    list: Option<&List>,
    dialog: &adw::Dialog,
    change: Rc<RefCell<ImageChange>>,
) -> (adw::ActionRow, adw::Avatar) {
    let picture = adw::Avatar::new(48, list.map(|l| l.name.as_str()), true);
    if let Some(url) = list.and_then(|l| l.avatar.as_deref()) {
        let picture = picture.downgrade();
        crate::images::load(url, move |texture| {
            if let Some(picture) = picture.upgrade() {
                picture.set_custom_image(Some(texture));
            }
        });
    }
    let row = adw::ActionRow::builder()
        .title(gettext("Image"))
        .subtitle(gettext("Facultative. PNG, JPEG ou WebP."))
        .build();
    row.add_prefix(&picture);
    let choose = gtk::Button::builder()
        .label(gettext("Choisir…"))
        .valign(gtk::Align::Center)
        .build();
    let remove = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text(gettext("Retirer l'image"))
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .visible(list.is_some_and(|l| l.avatar.is_some()))
        .build();
    remove.update_property(&[gtk::accessible::Property::Label(&gettext(
        "Retirer l'image",
    ))]);
    row.add_suffix(&choose);
    row.add_suffix(&remove);
    {
        let (picture, change) = (picture.clone(), change.clone());
        remove.connect_clicked(move |remove| {
            picture.set_custom_image(gdk::Paintable::NONE);
            *change.borrow_mut() = ImageChange::Remove;
            remove.set_visible(false);
        });
    }
    let dialog = dialog.downgrade();
    let (weak_row, weak_picture, weak_remove) =
        (row.downgrade(), picture.downgrade(), remove.downgrade());
    choose.connect_clicked(move |_| {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(&gettext("Images")));
        for mime in ["image/png", "image/jpeg", "image/webp"] {
            filter.add_mime_type(mime);
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let chooser = gtk::FileDialog::builder()
            .title(gettext("Choisir une image"))
            .modal(true)
            .filters(&filters)
            .default_filter(&filter)
            .build();
        let window = dialog
            .upgrade()
            .and_then(|d| d.root())
            .and_downcast::<gtk::Window>();
        let (row, picture, remove, change) = (
            weak_row.clone(),
            weak_picture.clone(),
            weak_remove.clone(),
            change.clone(),
        );
        chooser.open(window.as_ref(), gio::Cancellable::NONE, move |result| {
            let Ok(file) = result else { return };
            glib::spawn_future_local(async move {
                let bytes = match file.load_bytes_future().await {
                    Ok((bytes, _)) => bytes.to_vec(),
                    Err(_) => Vec::new(),
                };
                let prepared = gio::spawn_blocking(move || photo::prepare(&bytes))
                    .await
                    .unwrap_or_else(|_| Err(String::new()));
                let texture = prepared.as_ref().ok().and_then(|image| {
                    gdk::Texture::from_bytes(&glib::Bytes::from(&image.bytes)).ok()
                });
                let (Some(row), Some(picture), Some(remove)) =
                    (row.upgrade(), picture.upgrade(), remove.upgrade())
                else {
                    return;
                };
                match (prepared, texture) {
                    (Ok(image), Some(texture)) => {
                        picture.set_custom_image(Some(&texture));
                        *change.borrow_mut() = ImageChange::Replace(image);
                        remove.set_visible(true);
                        row.set_subtitle(&gettext("Facultative. PNG, JPEG ou WebP."));
                    }
                    _ => row.set_subtitle(&gettext("Ce fichier n'est pas une image lisible.")),
                }
            });
        });
    });
    (row, picture)
}

/// Boîte « Nouvelle liste » ou « Modifier la liste ». `done` reçoit la
/// liste créée ou modifiée.
pub fn edit_dialog(
    parent: &impl IsA<gtk::Widget>,
    list: Option<&List>,
    host: &Host,
    done: impl Fn() + 'static,
) {
    let creating = list.is_none();
    let dialog = adw::Dialog::builder()
        .title(if creating {
            gettext("Nouvelle liste")
        } else {
            gettext("Modifier la liste")
        })
        .content_width(420)
        .build();
    let cancel = gtk::Button::with_label(&gettext("Annuler"));
    let save = gtk::Button::builder()
        .label(if creating {
            gettext("Créer")
        } else {
            gettext("Enregistrer")
        })
        .css_classes(["suggested-action"])
        .build();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&save);

    let name = adw::EntryRow::builder().title(gettext("Nom")).build();
    let description = adw::EntryRow::builder()
        .title(gettext("Description"))
        .build();
    let moderation = adw::SwitchRow::builder()
        .title(gettext("Liste de modération"))
        .subtitle(gettext(
            "Pour masquer ou bloquer ses comptes, sans fil de posts.",
        ))
        .visible(creating)
        .build();
    if let Some(list) = list {
        name.set_text(&list.name);
        description.set_text(&list.description);
    }
    let avatar_change: Rc<RefCell<ImageChange>> = Rc::default();
    let (image_row, picture) = image_row(list, &dialog, avatar_change.clone());
    {
        let picture = picture.clone();
        name.connect_changed(move |row| picture.set_text(Some(&row.text())));
    }
    let group = adw::PreferencesGroup::new();
    group.add(&image_row);
    group.add(&name);
    group.add(&description);
    group.add(&moderation);
    let error = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .accessible_role(gtk::AccessibleRole::Alert)
        .css_classes(["error"])
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&group);
    content.append(&error);
    let view = adw::ToolbarView::builder().content(&content).build();
    view.add_top_bar(&header);
    dialog.set_child(Some(&view));

    let refresh = {
        let (name, description, save) = (name.clone(), description.clone(), save.clone());
        move || {
            let n = grapheme_count(name.text().trim());
            let d = grapheme_count(description.text().trim());
            save.set_sensitive(n > 0 && n <= MAX_NAME && d <= MAX_DESCRIPTION);
        }
    };
    refresh();
    let refresh = Rc::new(refresh);
    for row in [&name, &description] {
        let refresh = refresh.clone();
        row.connect_changed(move |_| refresh());
    }
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    let uri = list.map(|l| l.uri.clone());
    let (session, toasts) = (host.session.clone(), host.toasts.clone());
    let done = Rc::new(done);
    let presented = dialog.clone();
    save.connect_clicked(move |save| {
        let Some(session) = session.clone() else {
            toasts.add_toast(adw::Toast::new(&gettext(
                "Mode démonstration : rien n'est publié.",
            )));
            dialog.close();
            return;
        };
        let new_name = name.text().trim().to_owned();
        let new_description = description.text().trim().to_owned();
        let new_avatar = avatar_change.borrow().clone();
        save.set_sensitive(false);
        let task = match uri.clone() {
            Some(uri) => runtime().spawn(async move {
                Box::pin(update_list(
                    session,
                    uri,
                    new_name,
                    new_description,
                    new_avatar,
                ))
                .await
            }),
            None => {
                let moderation = moderation.is_active();
                runtime().spawn(async move {
                    Box::pin(create_list(
                        session,
                        new_name,
                        new_description,
                        moderation,
                        new_avatar,
                    ))
                    .await
                    .map(|_| ())
                })
            }
        };
        let (dialog, error, save, done, toasts) = (
            dialog.clone(),
            error.clone(),
            save.clone(),
            done.clone(),
            toasts.clone(),
        );
        glib::spawn_future_local(async move {
            match task
                .await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))))
            {
                Ok(()) => {
                    toasts.add_toast(adw::Toast::new(&gettext("Liste enregistrée.")));
                    dialog.close();
                    done();
                }
                Err(failure) => {
                    error.set_label(&failure.message);
                    error.set_visible(true);
                    save.set_sensitive(true);
                }
            }
        });
    });
    presented.present(Some(parent));
}

/// Lance `task` et affiche `success` (ou l'erreur) ; puis `then`.
fn run<T: Send + 'static>(
    host: &Host,
    task: impl std::future::Future<Output = Result<T, ApiError>> + Send + 'static,
    success: String,
    then: impl FnOnce(T) + 'static,
) {
    let task = runtime().spawn(task);
    let (toasts, expired) = (host.toasts.clone(), host.expired.clone());
    glib::spawn_future_local(async move {
        match task
            .await
            .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))))
        {
            Ok(value) => {
                toasts.add_toast(adw::Toast::new(&success));
                then(value);
            }
            Err(error) if error.expired => expired(error.message),
            Err(error) => toasts.add_toast(adw::Toast::new(&error.message)),
        }
    });
}

/// Bouton de menu d'une liste, pour l'en-tête de sa page : membres, et
/// selon qu'elle est à moi ou non, modifier et supprimer, ou masquer et
/// bloquer ses comptes (listes de modération). `changed` est appelé après
/// chaque changement.
pub fn menu_button(list: &List, host: &Host, changed: Rc<dyn Fn()>) -> gtk::MenuButton {
    let mine = host.me.as_deref() == Some(list.creator.as_str()) || host.session.is_none();
    let menu = gio::Menu::new();
    let actions = gio::SimpleActionGroup::new();
    let button = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text(gettext("Options de la liste"))
        .menu_model(&menu)
        .build();
    button.update_property(&[gtk::accessible::Property::Label(&gettext(
        "Options de la liste",
    ))]);
    button.insert_action_group("list", Some(&actions));

    let add = |name: &str, handler: Box<dyn Fn()>| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| handler());
        actions.add_action(&action);
    };
    if !list.moderation && host.session.is_some() {
        let pinned =
            crate::feeds::list_pinned(&crate::moderation::current_preferences(), &list.uri);
        menu.append(
            Some(&if pinned {
                gettext("Retirer des colonnes")
            } else {
                gettext("Épingler comme colonne")
            }),
            Some("list.pin"),
        );
        let (list, host, button) = (list.clone(), host.clone(), button.clone());
        add(
            "pin",
            Box::new(move || {
                let Some(session) = host.session.clone() else {
                    return;
                };
                let uri = list.uri.clone();
                let button = button.downgrade();
                run(
                    &host,
                    async move {
                        crate::moderation::update_preferences(session, move |p| {
                            crate::feeds::Change::PinList(uri, !pinned).apply(p)
                        })
                        .await
                    },
                    if pinned {
                        gettext("Liste retirée des colonnes.")
                    } else {
                        gettext("Liste épinglée : « Ajouter une colonne » la propose.")
                    },
                    move |preferences| {
                        crate::moderation::set_preferences(preferences);
                        if let Some(button) = button.upgrade() {
                            let _ = button.activate_action("win.reload-pinned", None);
                        }
                    },
                );
            }),
        );
    }
    if !list.moderation {
        menu.append(Some(&gettext("Membres")), Some("list.members"));
        let (list, host) = (list.clone(), host.clone());
        add(
            "members",
            Box::new(move || (host.push)(members_page(&list, &host, Rc::new(|| {})))),
        );
    }
    if mine {
        menu.append(Some(&gettext("Modifier la liste…")), Some("list.edit"));
        menu.append(Some(&gettext("Supprimer la liste…")), Some("list.delete"));
        {
            let (list, host, changed, button) =
                (list.clone(), host.clone(), changed.clone(), button.clone());
            add(
                "edit",
                Box::new(move || {
                    let changed = changed.clone();
                    edit_dialog(&button, Some(&list), &host, move || changed());
                }),
            );
        }
        let (list, host, changed, button) =
            (list.clone(), host.clone(), changed.clone(), button.clone());
        add(
            "delete",
            Box::new(move || confirm_delete(&button, &list, &host, changed.clone())),
        );
    } else if list.moderation {
        let mute_label = if list.muted {
            gettext("Ne plus masquer ces comptes")
        } else {
            gettext("Masquer ces comptes")
        };
        menu.append(Some(&mute_label), Some("list.mute"));
        let block_label = if list.blocked.is_some() {
            gettext("Ne plus bloquer ces comptes")
        } else {
            gettext("Bloquer ces comptes")
        };
        menu.append(Some(&block_label), Some("list.block"));
        {
            let (list, host, changed) = (list.clone(), host.clone(), changed.clone());
            add(
                "mute",
                Box::new(move || {
                    let Some(session) = host.session.clone() else {
                        return;
                    };
                    let muted = !list.muted;
                    let changed = changed.clone();
                    run(
                        &host,
                        set_muted(session, list.uri.clone(), muted),
                        if muted {
                            gettext("Les comptes de cette liste sont masqués.")
                        } else {
                            gettext("Les comptes de cette liste ne sont plus masqués.")
                        },
                        move |()| changed(),
                    );
                }),
            );
        }
        let (list, host, changed) = (list.clone(), host.clone(), changed.clone());
        add(
            "block",
            Box::new(move || {
                let Some(session) = host.session.clone() else {
                    return;
                };
                let changed = changed.clone();
                match list.blocked.clone() {
                    Some(record) => run(
                        &host,
                        api::delete(session, record),
                        gettext("Les comptes de cette liste ne sont plus bloqués."),
                        move |()| changed(),
                    ),
                    None => run(
                        &host,
                        block(session, list.uri.clone()),
                        gettext("Les comptes de cette liste sont bloqués."),
                        move |_| changed(),
                    ),
                }
            }),
        );
    }
    button
}

fn confirm_delete(parent: &gtk::MenuButton, list: &List, host: &Host, changed: Rc<dyn Fn()>) {
    let confirm = adw::AlertDialog::new(
        Some(&gettext("Supprimer cette liste ?")),
        Some(&gettext(
            "Elle disparaîtra pour tout le monde. Ses comptes ne sont pas prévenus.",
        )),
    );
    confirm.add_responses(&[
        ("cancel", &gettext("Annuler")),
        ("delete", &gettext("Supprimer")),
    ]);
    confirm.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    confirm.set_default_response(Some("cancel"));
    confirm.set_close_response("cancel");
    let (list, host, weak) = (list.clone(), host.clone(), parent.downgrade());
    confirm.connect_response(Some("delete"), move |_, _| {
        let Some(session) = host.session.clone() else {
            host.toasts.add_toast(adw::Toast::new(&gettext(
                "Mode démonstration : rien n'est publié.",
            )));
            return;
        };
        let (changed, weak) = (changed.clone(), weak.clone());
        run(
            &host,
            api::delete(session, list.uri.clone()),
            gettext("Liste supprimée."),
            move |()| {
                // Retour à la page précédente : celle-ci n'existe plus.
                if let Some(button) = weak.upgrade() {
                    let _ = button.activate_action("navigation.pop", None);
                }
                changed();
            },
        );
    });
    confirm.present(Some(parent));
}

/// Membres d'une liste. Sur mes listes, chaque compte a « Retirer de la
/// liste ».
pub fn members_page(list: &List, host: &Host, changed: Rc<dyn Fn()>) -> adw::NavigationPage {
    let rows = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .valign(gtk::Align::Start)
        .css_classes(["boxed-list"])
        .build();
    rows.update_property(&[gtk::accessible::Property::Label(&list.name)]);
    let empty = adw::StatusPage::builder()
        .icon_name("view-list-bullet-symbolic")
        .title(gettext("Chargement…"))
        .css_classes(["compact"])
        .build();
    let stack = gtk::Stack::new();
    stack.add_named(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(crate::columns::COLUMN_MAX_WIDTH + 24)
                    .margin_top(12)
                    .margin_bottom(24)
                    .margin_start(12)
                    .margin_end(12)
                    .child(&rows)
                    .build(),
            )
            .build(),
        Some("list"),
    );
    stack.add_named(&empty, Some("status"));
    stack.set_visible_child_name("status");
    let header = adw::HeaderBar::builder()
        .title_widget(&adw::WindowTitle::new(&list.name, &gettext("Membres")))
        .build();
    if list.moderation {
        header.pack_end(&menu_button(list, host, changed));
    }
    let root = adw::ToolbarView::builder().content(&stack).build();
    root.add_top_bar(&header);
    let page = adw::NavigationPage::builder()
        .title(&list.name)
        .child(&root)
        .build();

    let mine = host.me.as_deref() == Some(list.creator.as_str());
    let fill = {
        let (rows, stack, empty, host) = (rows.clone(), stack.clone(), empty.clone(), host.clone());
        move |members: Result<Vec<(Actor, Option<String>)>, ApiError>| match members {
            Ok(members) if !members.is_empty() => {
                for (actor, item) in members {
                    let row = actor_row(&actor, host.session.clone(), &host.toasts);
                    if let (true, Some(item), Some(session)) = (mine, item, host.session.clone()) {
                        let remove = gtk::Button::builder()
                            .icon_name("list-remove-symbolic")
                            .tooltip_text(gettext("Retirer de la liste"))
                            .valign(gtk::Align::Center)
                            .css_classes(["flat"])
                            .build();
                        remove.update_property(&[gtk::accessible::Property::Label(
                            &gettext("Retirer {} de la liste").replace("{}", &actor.name),
                        )]);
                        let (host, rows, row_weak) = (host.clone(), rows.clone(), row.downgrade());
                        remove.connect_clicked(move |_| {
                            let (rows, row_weak) = (rows.clone(), row_weak.clone());
                            run(
                                &host,
                                api::delete(session.clone(), item.clone()),
                                gettext("Compte retiré de la liste."),
                                move |()| {
                                    if let Some(row) = row_weak.upgrade() {
                                        rows.remove(&row);
                                    }
                                },
                            );
                        });
                        row.add_suffix(&remove);
                    }
                    rows.append(&row);
                }
                stack.set_visible_child_name("list");
            }
            Ok(_) => empty.set_title(&gettext("Cette liste est vide.")),
            Err(failure) => empty.set_title(&failure.message),
        }
    };
    let Some(session) = host.session.clone() else {
        fill(Ok(data::demo_actors()
            .into_iter()
            .map(|a| (a, None))
            .collect()));
        return page;
    };
    let task = runtime().spawn(items(session, list.uri.clone()));
    glib::spawn_future_local(async move {
        let result = task
            .await
            .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu."))));
        fill(result.map(|items| items.into_iter().map(|(a, i)| (a, Some(i))).collect()));
    });
    page
}

/// Boîte « Ajouter à une liste » : mes listes, cochées si `actor` (DID) y
/// est. Cocher ou décocher l'ajoute ou le retire aussitôt.
pub fn memberships_dialog(parent: &impl IsA<gtk::Widget>, actor: &str, name: &str, host: &Host) {
    let dialog = adw::Dialog::builder()
        .title(gettext("Listes"))
        .content_width(420)
        .content_height(480)
        .build();
    let rows = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .valign(gtk::Align::Start)
        .css_classes(["boxed-list"])
        .build();
    rows.update_property(&[gtk::accessible::Property::Label(
        &gettext("Listes de {}").replace("{}", name),
    )]);
    let status = adw::StatusPage::builder()
        .title(gettext("Chargement…"))
        .css_classes(["compact"])
        .build();
    let stack = gtk::Stack::new();
    stack.add_named(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(
                &adw::Clamp::builder()
                    .margin_top(12)
                    .margin_bottom(12)
                    .margin_start(12)
                    .margin_end(12)
                    .child(&rows)
                    .build(),
            )
            .build(),
        Some("list"),
    );
    stack.add_named(&status, Some("status"));
    stack.set_visible_child_name("status");
    let header = adw::HeaderBar::builder()
        .title_widget(&adw::WindowTitle::new(
            &gettext("Ajouter à une liste"),
            &format!("@{name}"),
        ))
        .build();
    let view = adw::ToolbarView::builder().content(&stack).build();
    view.add_top_bar(&header);
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));

    let fill = {
        let (rows, stack, status, host, actor) = (
            rows.clone(),
            stack.clone(),
            status.clone(),
            host.clone(),
            actor.to_owned(),
        );
        move |result: Result<Vec<(List, Option<String>)>, ApiError>| match result {
            Ok(lists) if !lists.is_empty() => {
                for (list, item) in lists {
                    let check = gtk::CheckButton::builder()
                        .active(item.is_some())
                        .valign(gtk::Align::Center)
                        .build();
                    let row = adw::ActionRow::builder()
                        .title(&list.name)
                        .subtitle(if list.moderation {
                            gettext("Liste de modération")
                        } else {
                            gettext("Liste de comptes")
                        })
                        .use_markup(false)
                        .activatable_widget(&check)
                        .build();
                    row.add_prefix(&check);
                    let item = Rc::new(std::cell::RefCell::new(item));
                    let (host, actor) = (host.clone(), actor.clone());
                    check.connect_toggled(move |check| {
                        let Some(session) = host.session.clone() else {
                            return;
                        };
                        let current = item.borrow().clone();
                        match (check.is_active(), current) {
                            (true, None) => {
                                let item = item.clone();
                                run(
                                    &host,
                                    add_member(session, list.uri.clone(), actor.clone()),
                                    gettext("Ajouté à « {} ».").replace("{}", &list.name),
                                    move |uri| {
                                        item.replace(Some(uri));
                                    },
                                );
                            }
                            (false, Some(uri)) => {
                                let item = item.clone();
                                run(
                                    &host,
                                    api::delete(session, uri),
                                    gettext("Retiré de « {} ».").replace("{}", &list.name),
                                    move |()| {
                                        item.replace(None);
                                    },
                                );
                            }
                            _ => {}
                        }
                    });
                    rows.append(&row);
                }
                stack.set_visible_child_name("list");
            }
            Ok(_) => status.set_title(&gettext("Vous n'avez pas encore de liste.")),
            Err(failure) => status.set_title(&failure.message),
        }
    };
    let Some(session) = host.session.clone() else {
        fill(Ok(data::demo_lists()
            .into_iter()
            .map(|l| (l, None))
            .collect()));
        return;
    };
    let task = runtime().spawn(memberships(session, actor.to_owned()));
    glib::spawn_future_local(async move {
        fill(
            task.await
                .unwrap_or_else(|_| Err(ApiError::new(gettext("Chargement interrompu.")))),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_pack_feeds_are_set() {
        let mut record = json!({"name": "Pack", "feeds": [{"uri": "a"}]});
        let feeds: Vec<String> = ["b", "c", "d", "e"].map(String::from).to_vec();
        set_feeds(&mut record, &feeds);
        assert_eq!(
            record["feeds"],
            json!([{"uri": "b"}, {"uri": "c"}, {"uri": "d"}])
        );
        set_feeds(&mut record, &[]);
        assert!(record.get("feeds").is_none());
        assert_eq!(record["name"], "Pack");
    }

    #[test]
    fn list_avatar_is_kept_replaced_or_removed() {
        let mut record = json!({"name": "Amis", "avatar": {"ref": "ancien"}});
        apply_avatar(&mut record, &ImageChange::Keep, None);
        assert_eq!(record["avatar"]["ref"], "ancien");
        let image = Prepared {
            bytes: vec![1],
            mime: "image/png",
            width: 1,
            height: 1,
        };
        apply_avatar(
            &mut record,
            &ImageChange::Replace(image),
            Some(json!({"ref": "nouveau"})),
        );
        assert_eq!(record["avatar"]["ref"], "nouveau");
        apply_avatar(&mut record, &ImageChange::Remove, None);
        assert!(record.get("avatar").is_none());
        assert_eq!(record["name"], "Amis");
    }

    #[test]
    fn starter_pack_records_are_valid() {
        let record = starter_pack_record(
            "GNOME",
            "",
            "at://did:plc:a/app.bsky.graph.list/l",
            "2026-11-23T10:00:00.000Z",
        );
        assert!(record.get("description").is_none());
        let _: atrium_api::app::bsky::graph::starterpack::Record =
            serde_json::from_value(record).unwrap();
    }

    #[test]
    fn list_records_are_valid() {
        let record = list_record("Amis", "", true, "2026-11-23T10:00:00.000Z");
        assert_eq!(record["purpose"], "app.bsky.graph.defs#modlist");
        assert!(record.get("description").is_none());
        let _: atrium_api::app::bsky::graph::list::Record = serde_json::from_value(record).unwrap();
        let record = list_record("GNOME", "Le bureau", false, "2026-11-23T10:00:00.000Z");
        assert_eq!(record["purpose"], "app.bsky.graph.defs#curatelist");
        let _: atrium_api::app::bsky::graph::list::Record = serde_json::from_value(record).unwrap();
    }
}
