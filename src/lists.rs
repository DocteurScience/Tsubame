//! Listes : créer, renommer, supprimer, gérer les membres ; s'abonner aux
//! listes de modération des autres (masquer ou bloquer leurs comptes).

use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use atrium_api::types::string::{Nsid, RecordKey};
use gettextrs::gettext;
use gtk::{gio, glib};
use serde_json::{Value, json};

use crate::api::{self, ApiError, invalid_actor, request_failed};
use crate::auth::Session;
use crate::data::{self, Actor, List};
use crate::explore::actor_row;
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

pub async fn create_list(
    session: Arc<Session>,
    name: String,
    description: String,
    moderation: bool,
) -> Result<String, ApiError> {
    let record = list_record(&name, &description, moderation, &now());
    create_record(&session, LIST, record).await
}

/// Renomme une liste et change sa description, en gardant le reste de son
/// enregistrement, s'il n'a pas changé entre-temps.
pub async fn update_list(
    session: Arc<Session>,
    uri: String,
    name: String,
    description: String,
) -> Result<(), ApiError> {
    use atrium_api::com::atproto::repo::{get_record, put_record};
    let (repo, _, rkey) = api::parse_record_uri(&uri).ok_or_else(invalid_actor)?;
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
                collection: Nsid::new(LIST.to_owned()).map_err(|_| invalid_actor())?,
                repo: repo.parse().map_err(|_| invalid_actor())?,
                rkey: rkey.clone(),
            }
            .into(),
        )
        .await
        .map_err(request_failed)?;
    let mut record = serde_json::to_value(&current.data.value).unwrap_or_default();
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
    api.api
        .com
        .atproto
        .repo
        .put_record(
            put_record::InputData {
                collection: Nsid::new(LIST.to_owned()).map_err(|_| invalid_actor())?,
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
    let group = adw::PreferencesGroup::new();
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
        save.set_sensitive(false);
        let task = match uri.clone() {
            Some(uri) => runtime().spawn(async move {
                Box::pin(update_list(session, uri, new_name, new_description)).await
            }),
            None => {
                let moderation = moderation.is_active();
                runtime().spawn(async move {
                    Box::pin(create_list(session, new_name, new_description, moderation))
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
