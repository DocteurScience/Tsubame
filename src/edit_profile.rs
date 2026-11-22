//! Modifier son profil : nom affiché, bio, avatar et bannière.
//!
//! Le profil est l'enregistrement `app.bsky.actor.profile/self`. On le
//! relit, on ne change que ces quatre champs, on garde les autres (post
//! épinglé, étiquettes…) et on le réécrit à condition qu'il n'ait pas
//! changé entre-temps (`swapRecord`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use atrium_api::types::string::{Cid, Nsid, RecordKey};
use gettextrs::gettext;
use gtk::{gdk, gio, glib};
use serde_json::{Value, json};

use crate::api::{ApiError, request_failed};
use crate::auth::Session;
use crate::data::Profile;
use crate::images;
use crate::photo::{self, Prepared};
use crate::richtext::grapheme_count;
use crate::runtime::runtime;

const COLLECTION: &str = "app.bsky.actor.profile";
pub const MAX_NAME: usize = 64;
pub const MAX_BIO: usize = 256;

/// Ce qu'il advient d'une image du profil.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ImageChange {
    #[default]
    Keep,
    Remove,
    Replace(Prepared),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileEdit {
    pub name: String,
    pub bio: String,
    pub avatar: ImageChange,
    pub banner: ImageChange,
}

/// Applique la modification à l'enregistrement. `avatar` et `banner` sont
/// les blobs envoyés pour les images remplacées.
pub fn apply(record: &mut Value, edit: &ProfileEdit, avatar: Option<Value>, banner: Option<Value>) {
    if !record.is_object() {
        *record = json!({});
    }
    let map = record.as_object_mut().expect("objet");
    map.insert("$type".into(), json!(COLLECTION));
    for (key, value) in [
        ("displayName", edit.name.trim()),
        ("description", edit.bio.trim()),
    ] {
        if value.is_empty() {
            map.remove(key);
        } else {
            map.insert(key.into(), json!(value));
        }
    }
    for (key, change, blob) in [
        ("avatar", &edit.avatar, avatar),
        ("banner", &edit.banner, banner),
    ] {
        match (change, blob) {
            (ImageChange::Keep, _) => {}
            (ImageChange::Remove, _) => {
                map.remove(key);
            }
            (ImageChange::Replace(_), Some(blob)) => {
                map.insert(key.into(), blob);
            }
            (ImageChange::Replace(_), None) => {}
        }
    }
}

/// Le profil actuel (enregistrement et son CID), ou un profil vide.
async fn current(session: &Session) -> Result<(Value, Option<Cid>), ApiError> {
    use atrium_api::com::atproto::repo::get_record;
    let api = session.api().await?;
    let result = Box::pin(
        api.api.com.atproto.repo.get_record(
            get_record::ParametersData {
                cid: None,
                collection: Nsid::new(COLLECTION.to_owned()).expect("NSID"),
                repo: session
                    .account
                    .did
                    .parse()
                    .map_err(|_| ApiError::new(gettext("Compte invalide.")))?,
                rkey: RecordKey::new("self".to_owned()).expect("clé"),
            }
            .into(),
        ),
    )
    .await;
    match result {
        Ok(output) => Ok((
            serde_json::to_value(&output.data.value).unwrap_or_default(),
            output.data.cid,
        )),
        // Pas encore de profil : on part de rien.
        Err(error) if error.to_string().contains("RecordNotFound") => Ok((json!({}), None)),
        Err(error) => Err(request_failed(error)),
    }
}

/// Nom affiché et bio tels qu'enregistrés (sans l'identifiant à la place
/// du nom).
pub async fn load(session: Arc<Session>) -> Result<(String, String), ApiError> {
    let (record, _) = Box::pin(current(&session)).await?;
    let text = |key: &str| {
        record
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Ok((text("displayName"), text("description")))
}

pub async fn save(session: Arc<Session>, edit: ProfileEdit) -> Result<(), ApiError> {
    use atrium_api::com::atproto::repo::put_record;
    let (mut record, cid) = Box::pin(current(&session)).await?;
    let api = session.api().await?;
    let mut blobs = Vec::new();
    for change in [&edit.avatar, &edit.banner] {
        blobs.push(match change {
            ImageChange::Replace(image) => {
                let output = api
                    .api
                    .com
                    .atproto
                    .repo
                    .upload_blob(image.bytes.clone())
                    .await
                    .map_err(|e| {
                        ApiError::new(
                            gettext("Une image n'a pas pu être envoyée : {}")
                                .replace("{}", &e.to_string()),
                        )
                    })?;
                serde_json::to_value(&output.data.blob).ok()
            }
            _ => None,
        });
    }
    let banner = blobs.pop().flatten();
    let avatar = blobs.pop().flatten();
    apply(&mut record, &edit, avatar, banner);
    let record = serde_json::from_value(record)
        .map_err(|_| ApiError::new(gettext("Le profil n'a pas pu être enregistré.")))?;
    Box::pin(
        api.api.com.atproto.repo.put_record(
            put_record::InputData {
                collection: Nsid::new(COLLECTION.to_owned()).expect("NSID"),
                record,
                repo: session
                    .account
                    .did
                    .parse()
                    .map_err(|_| ApiError::new(gettext("Compte invalide.")))?,
                rkey: RecordKey::new("self".to_owned()).expect("clé"),
                swap_commit: None,
                swap_record: cid,
                validate: None,
            }
            .into(),
        ),
    )
    .await
    .map_err(request_failed)?;
    Ok(())
}

/// Compteur « n / max », en couleur d'erreur au-delà.
fn update_counter(counter: &gtk::Label, text: &str, max: usize) -> bool {
    let count = grapheme_count(text);
    counter.set_label(&format!("{count} / {max}"));
    let over = count > max;
    if over {
        counter.add_css_class("error");
    } else {
        counter.remove_css_class("error");
    }
    !over
}

/// Boîte « Modifier le profil ». `on_saved` est appelé après
/// l'enregistrement (ou tout de suite en démonstration).
pub fn present(
    parent: &impl IsA<gtk::Widget>,
    session: Option<Arc<Session>>,
    profile: &Profile,
    on_saved: impl Fn() + 'static,
) {
    let cancel = gtk::Button::with_label(&gettext("Annuler"));
    let save_button = gtk::Button::builder()
        .label(gettext("Enregistrer"))
        .css_classes(["suggested-action"])
        .build();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&save_button);

    let edit = Rc::new(RefCell::new(ProfileEdit::default()));

    // Bannière et avatar, chacun avec « Changer » et « Retirer ».
    let banner = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .height_request(120)
        .build();
    banner.update_property(&[gtk::accessible::Property::Label(&gettext("Bannière"))]);
    let banner_frame = gtk::Overlay::builder()
        .child(&banner)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["profile-banner-frame"])
        .build();
    let avatar = adw::Avatar::builder()
        .size(88)
        .text(&profile.name)
        .show_initials(true)
        .build();
    if let Some(url) = &profile.avatar {
        let weak = avatar.downgrade();
        images::load(url, move |texture| {
            if let Some(avatar) = weak.upgrade() {
                avatar.set_custom_image(Some(texture));
            }
        });
    }
    if let Some(url) = &profile.banner {
        let weak = banner.downgrade();
        images::load(url, move |texture| {
            if let Some(banner) = weak.upgrade() {
                banner.set_paintable(Some(texture));
            }
        });
    }
    let image_buttons = |change_label: String, remove_label: String| {
        let change = gtk::Button::builder()
            .icon_name("camera-photo-symbolic")
            .tooltip_text(&change_label)
            .css_classes(["circular", "osd"])
            .build();
        change.update_property(&[gtk::accessible::Property::Label(&change_label)]);
        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text(&remove_label)
            .css_classes(["circular", "osd"])
            .build();
        remove.update_property(&[gtk::accessible::Property::Label(&remove_label)]);
        let row = gtk::Box::builder()
            .spacing(6)
            .halign(gtk::Align::End)
            .valign(gtk::Align::End)
            .margin_end(8)
            .margin_bottom(8)
            .build();
        row.append(&change);
        row.append(&remove);
        (row, change, remove)
    };
    let (banner_buttons, change_banner, remove_banner) = image_buttons(
        gettext("Changer la bannière"),
        gettext("Retirer la bannière"),
    );
    banner_frame.add_overlay(&banner_buttons);
    let (avatar_buttons, change_avatar, remove_avatar) =
        image_buttons(gettext("Changer l'avatar"), gettext("Retirer l'avatar"));
    avatar_buttons.set_halign(gtk::Align::Start);
    avatar_buttons.set_valign(gtk::Align::Center);
    avatar_buttons.set_margin_bottom(0);
    let avatar_row = gtk::Box::builder().spacing(12).build();
    avatar_row.append(&avatar);
    avatar_row.append(&avatar_buttons);

    let error = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .css_classes(["error"])
        .build();

    // Texte : nom affiché et bio, avec leurs compteurs.
    let name = adw::EntryRow::builder()
        .title(gettext("Nom affiché"))
        .build();
    let name_counter = gtk::Label::builder()
        .css_classes(["dim-label", "caption", "numeric"])
        .build();
    name.add_suffix(&name_counter);
    let names = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    names.append(&name);
    let bio = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .top_margin(10)
        .bottom_margin(10)
        .left_margin(12)
        .right_margin(12)
        .height_request(110)
        .css_classes(["card"])
        .build();
    bio.update_property(&[gtk::accessible::Property::Label(&gettext("Bio"))]);
    let bio_title = gtk::Label::builder()
        .label(gettext("Bio"))
        .xalign(0.0)
        .css_classes(["heading"])
        .mnemonic_widget(&bio)
        .build();
    let bio_counter = gtk::Label::builder()
        .halign(gtk::Align::End)
        .css_classes(["dim-label", "caption", "numeric"])
        .build();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(24)
        .margin_start(12)
        .margin_end(12)
        .build();
    for widget in [
        banner_frame.upcast_ref::<gtk::Widget>(),
        avatar_row.upcast_ref(),
        error.upcast_ref(),
        names.upcast_ref(),
        bio_title.upcast_ref(),
        bio.upcast_ref(),
        bio_counter.upcast_ref(),
    ] {
        content.append(widget);
    }
    let form = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(
            &adw::Clamp::builder()
                .maximum_size(520)
                .child(&content)
                .build(),
        )
        .build();
    let stack = gtk::Stack::new();
    stack.add_named(&form, Some("form"));
    stack.add_named(
        &adw::StatusPage::builder()
            .title(gettext("Chargement…"))
            .css_classes(["compact"])
            .build(),
        Some("loading"),
    );
    let toolbar = adw::ToolbarView::builder().content(&stack).build();
    toolbar.add_top_bar(&header);
    let dialog = adw::Dialog::builder()
        .title(gettext("Modifier le profil"))
        .content_width(520)
        .content_height(640)
        .child(&toolbar)
        .build();

    let validate = {
        let (name, bio, name_counter, bio_counter, save_button) = (
            name.clone(),
            bio.clone(),
            name_counter.clone(),
            bio_counter.clone(),
            save_button.clone(),
        );
        move || {
            let buffer = bio.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let name_ok = update_counter(&name_counter, &name.text(), MAX_NAME);
            let bio_ok = update_counter(&bio_counter, &text, MAX_BIO);
            save_button.set_sensitive(name_ok && bio_ok);
        }
    };
    let validate = Rc::new(validate);
    let fill = {
        let (name, bio, validate) = (name.clone(), bio.clone(), validate.clone());
        move |display_name: &str, description: &str| {
            name.set_text(display_name);
            bio.buffer().set_text(description);
            validate();
        }
    };
    {
        let validate = validate.clone();
        name.connect_changed(move |_| validate());
    }
    {
        let validate = validate.clone();
        bio.buffer().connect_changed(move |_| validate());
    }

    // Choisir une image : préparée hors de la boucle GTK (taille, poids,
    // métadonnées retirées), puis montrée.
    let choose = {
        let (dialog, edit, error) = (dialog.clone(), edit.clone(), error.clone());
        move |banner_image: bool, show: Rc<dyn Fn(&gdk::Texture)>| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(&gettext("Images")));
            for mime in ["image/png", "image/jpeg", "image/webp"] {
                filter.add_mime_type(mime);
            }
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let chooser = gtk::FileDialog::builder()
                .title(if banner_image {
                    gettext("Choisir une bannière")
                } else {
                    gettext("Choisir un avatar")
                })
                .modal(true)
                .filters(&filters)
                .default_filter(&filter)
                .build();
            let window = dialog.root().and_downcast::<gtk::Window>();
            let (edit, error) = (edit.clone(), error.clone());
            chooser.open(window.as_ref(), gio::Cancellable::NONE, move |result| {
                let Ok(file) = result else { return };
                let (edit, error, show) = (edit.clone(), error.clone(), show.clone());
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
                    match (prepared, texture) {
                        (Ok(image), Some(texture)) => {
                            error.set_visible(false);
                            show(&texture);
                            let mut edit = edit.borrow_mut();
                            if banner_image {
                                edit.banner = ImageChange::Replace(image);
                            } else {
                                edit.avatar = ImageChange::Replace(image);
                            }
                        }
                        _ => {
                            error.set_label(&gettext("Ce fichier n'est pas une image lisible."));
                            error.set_visible(true);
                        }
                    }
                });
            });
        }
    };
    let choose = Rc::new(choose);
    {
        let (choose, avatar) = (choose.clone(), avatar.clone());
        change_avatar.connect_clicked(move |_| {
            let avatar = avatar.clone();
            choose(
                false,
                Rc::new(move |texture| avatar.set_custom_image(Some(texture))),
            );
        });
    }
    {
        let (choose, banner) = (choose.clone(), banner.clone());
        change_banner.connect_clicked(move |_| {
            let banner = banner.clone();
            choose(
                true,
                Rc::new(move |texture| banner.set_paintable(Some(texture))),
            );
        });
    }
    {
        let (edit, avatar) = (edit.clone(), avatar.clone());
        remove_avatar.connect_clicked(move |_| {
            avatar.set_custom_image(None::<&gdk::Paintable>);
            edit.borrow_mut().avatar = ImageChange::Remove;
        });
    }
    {
        let (edit, banner) = (edit.clone(), banner.clone());
        remove_banner.connect_clicked(move |_| {
            banner.set_paintable(None::<&gdk::Paintable>);
            edit.borrow_mut().banner = ImageChange::Remove;
        });
    }

    let weak = dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(dialog) = weak.upgrade() {
            dialog.close();
        }
    });
    let on_saved = Rc::new(on_saved);
    {
        let (weak, session, edit, error) = (
            dialog.downgrade(),
            session.clone(),
            edit.clone(),
            error.clone(),
        );
        let (name, bio, header) = (name.clone(), bio.clone(), header.clone());
        save_button.connect_clicked(move |button| {
            let Some(dialog) = weak.upgrade() else { return };
            let buffer = bio.buffer();
            let mut change = edit.borrow().clone();
            change.name = name.text().to_string();
            change.bio = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .to_string();
            let Some(session) = session.clone() else {
                dialog.close();
                on_saved();
                return;
            };
            button.set_sensitive(false);
            let spinner = gtk::Spinner::builder().spinning(true).build();
            header.set_title_widget(Some(&spinner));
            let task = runtime().spawn(save(session, change));
            let (on_saved, button, error, header) = (
                on_saved.clone(),
                button.clone(),
                error.clone(),
                header.clone(),
            );
            glib::spawn_future_local(async move {
                let result = task
                    .await
                    .unwrap_or_else(|_| Err(ApiError::new(gettext("Requête interrompue."))));
                header.set_title_widget(None::<&gtk::Widget>);
                match result {
                    Ok(()) => {
                        dialog.close();
                        on_saved();
                    }
                    Err(failure) => {
                        button.set_sensitive(true);
                        error.set_label(&failure.message);
                        error.set_visible(true);
                    }
                }
            });
        });
    }

    // Valeurs de départ : telles qu'enregistrées, ou celles de la
    // démonstration.
    match session {
        None => fill(&profile.name, &profile.bio),
        Some(session) => {
            stack.set_visible_child_name("loading");
            save_button.set_sensitive(false);
            let task = runtime().spawn(load(session));
            let (stack, error, fallback) = (stack.clone(), error.clone(), profile.clone());
            glib::spawn_future_local(async move {
                match task.await {
                    Ok(Ok((display_name, description))) => fill(&display_name, &description),
                    Ok(Err(failure)) => {
                        fill(&fallback.name, &fallback.bio);
                        error.set_label(&failure.message);
                        error.set_visible(true);
                    }
                    Err(_) => fill(&fallback.name, &fallback.bio),
                }
                stack.set_visible_child_name("form");
            });
        }
    }
    dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_edited_fields_change() {
        let mut record = json!({
            "$type": COLLECTION,
            "displayName": "Ancien nom",
            "description": "Ancienne bio",
            "avatar": {"$type": "blob", "ref": {"$link": "a"}, "mimeType": "image/jpeg", "size": 1},
            "banner": {"$type": "blob", "ref": {"$link": "b"}, "mimeType": "image/jpeg", "size": 1},
            "pinnedPost": {"uri": "at://did:plc:moi/app.bsky.feed.post/1", "cid": "c"}
        });
        let edit = ProfileEdit {
            name: "  Nouveau nom ".into(),
            bio: String::new(),
            avatar: ImageChange::Replace(Prepared {
                bytes: vec![1],
                mime: "image/png",
                width: 1,
                height: 1,
            }),
            banner: ImageChange::Remove,
        };
        let blob =
            json!({"$type": "blob", "ref": {"$link": "n"}, "mimeType": "image/png", "size": 1});
        apply(&mut record, &edit, Some(blob.clone()), None);
        assert_eq!(record["displayName"], "Nouveau nom");
        assert!(record.get("description").is_none());
        assert_eq!(record["avatar"], blob);
        assert!(record.get("banner").is_none());
        assert_eq!(record["pinnedPost"]["cid"], "c");

        // Sans profil existant : un enregistrement complet est créé.
        let mut empty = json!({});
        apply(
            &mut empty,
            &ProfileEdit {
                name: "Moi".into(),
                ..Default::default()
            },
            None,
            None,
        );
        assert_eq!(empty, json!({"$type": COLLECTION, "displayName": "Moi"}));
    }
}
