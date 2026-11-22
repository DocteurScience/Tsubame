//! Boîte de rédaction : nouveau post ou réponse (maquette
//! `docs/mockups/source/Compose.dc.html`).
//!
//! Texte, langue, compteur de caractères, images, réponse, citation,
//! réglages de réponse, brouillons. Les liens, mots-dièse et mentions
//! deviennent des facettes à la publication.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gdk, gio, glib};

use crate::api::{self, NewImage, NewPost};
use crate::auth::Session;
use crate::data::{Post, StrongRef};
use crate::drafts::{self, Draft};
use crate::photo;
use crate::richtext::{MAX_GRAPHEMES, grapheme_count};
use crate::runtime::runtime;

/// Posts ajoutés au fil, au plus, après le premier.
const MAX_FOLLOWUPS: usize = 9;

/// Post suivant du fil : sa ligne, son texte et son compteur.
struct Followup {
    row: gtk::Box,
    view: gtk::TextView,
    counter: gtk::Label,
}

impl Followup {
    fn text(&self) -> String {
        let buffer = self.view.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string()
    }
}

/// Langues proposées, sous leur propre nom.
const LANGUAGES: [(&str, &str); 6] = [
    ("fr", "Français"),
    ("en", "English"),
    ("es", "Español"),
    ("de", "Deutsch"),
    ("it", "Italiano"),
    ("pt", "Português"),
];

/// Langue de l'interface si elle est proposée, sinon le français.
fn default_language() -> &'static str {
    glib::language_names()
        .iter()
        .filter_map(|name| name.split(['_', '.', '@']).next().map(str::to_owned))
        .find_map(|code| LANGUAGES.iter().find(|(c, _)| *c == code).map(|(c, _)| *c))
        .unwrap_or("fr")
}

fn language_name(code: &str) -> &'static str {
    LANGUAGES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, name)| *name)
        .unwrap_or("Français")
}

/// Nombre de caractères, `Err` si le post ne peut pas être publié : vide
/// (ni texte ni image) ou trop long.
pub fn validate(text: &str, images: usize) -> Result<usize, usize> {
    let count = grapheme_count(text);
    if (text.trim().is_empty() && images == 0) || count > MAX_GRAPHEMES {
        Err(count)
    } else {
        Ok(count)
    }
}

/// Ouvre la boîte de rédaction. `on_published` reçoit l'adresse du post
/// auquel on a répondu, le cas échéant.
pub fn present(
    parent: &impl IsA<gtk::Widget>,
    toasts: &adw::ToastOverlay,
    session: Option<Arc<Session>>,
    reply_to: Option<Post>,
    quote: Option<Post>,
    on_published: impl Fn(Option<String>) + 'static,
) {
    let replying = reply_to.is_some();
    let quoting = usize::from(quote.is_some());
    let dialog = adw::Dialog::builder()
        .title(if replying {
            gettext("Répondre")
        } else if quoting > 0 {
            gettext("Citer")
        } else {
            gettext("Nouveau post")
        })
        .content_width(560)
        .content_height(400)
        .build();

    // En-tête : Annuler, titre, Publier.
    let cancel = gtk::Button::with_label(&gettext("Annuler"));
    let publish = gtk::Button::builder()
        .label(gettext("Publier"))
        .css_classes(["suggested-action"])
        .sensitive(false)
        .build();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&publish);

    // Brouillons (nouveau post seulement) : bouton visible tant que le
    // texte est vide.
    let can_draft = reply_to.is_none() && quote.is_none();
    let saved_drafts = Rc::new(RefCell::new(if can_draft {
        drafts::load()
    } else {
        Vec::new()
    }));
    let draft_id: Rc<RefCell<Option<String>>> = Rc::default();
    let drafts_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .width_request(320)
        .css_classes(["boxed-list"])
        .build();
    drafts_list.update_property(&[gtk::accessible::Property::Label(&gettext("Brouillons"))]);
    let drafts_popover = gtk::Popover::builder()
        .child(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .propagate_natural_height(true)
                .propagate_natural_width(true)
                .max_content_height(360)
                .child(&drafts_list)
                .build(),
        )
        .build();
    let drafts_button = gtk::MenuButton::builder()
        .label(gettext("Brouillons"))
        .popover(&drafts_popover)
        .visible(false)
        .build();
    header.pack_start(&drafts_button);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    // Post auquel on répond, ou post cité, en rappel.
    let reminder = reply_to
        .as_ref()
        .map(|post| (post, gettext("En réponse à {}")))
        .or_else(|| quote.as_ref().map(|post| (post, gettext("Citation de {}"))));
    if let Some((post, title)) = reminder {
        let quoted = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .css_classes(["card", "compose-quote"])
            .build();
        quoted.append(
            &gtk::Label::builder()
                .label(title.replace("{}", &post.name))
                .xalign(0.0)
                .css_classes(["caption-heading", "dim-label"])
                .build(),
        );
        quoted.append(
            &gtk::Label::builder()
                .label(&post.text)
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .lines(3)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["dim-label"])
                .build(),
        );
        content.append(&quoted);
    }

    // Zone de texte, avec une indication tant qu'elle est vide.
    let author = session
        .as_ref()
        .map(|s| s.account.handle.clone())
        .unwrap_or_default();
    let avatar = adw::Avatar::builder()
        .size(40)
        .text(&author)
        .show_initials(true)
        .valign(gtk::Align::Start)
        .build();
    let text = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .hexpand(true)
        .vexpand(true)
        .top_margin(8)
        .css_classes(["compose-text"])
        .build();
    let placeholder_text = if replying {
        gettext("Votre réponse")
    } else {
        gettext("Quoi de neuf ?")
    };
    text.update_property(&[gtk::accessible::Property::Label(&placeholder_text)]);
    let placeholder = gtk::Label::builder()
        .label(&placeholder_text)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .margin_top(8)
        .can_target(false)
        .css_classes(["dim-label"])
        .build();
    let editor = gtk::Overlay::builder().child(&text).build();
    editor.add_overlay(&placeholder);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.append(&avatar);
    row.append(&editor);
    content.append(&row);

    let error = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .accessible_role(gtk::AccessibleRole::Alert)
        .css_classes(["error"])
        .build();
    content.append(&error);

    // Images jointes : vignettes et rappel du texte alternatif.
    let attachments: Rc<RefCell<Vec<Rc<Attachment>>>> = Rc::default();
    let tiles = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    let images_area = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_start(50)
        .visible(false)
        .build();
    images_area.append(&tiles);
    images_area.append(
        &gtk::Label::builder()
            .label(gettext(
                "Décrivez chaque image pour les personnes qui utilisent un lecteur d'écran.",
            ))
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .build(),
    );
    content.insert_child_after(&images_area, Some(&row));

    // Fil : posts suivants (texte seul), publiés en réponses successives.
    let thread_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build();
    let add_to_thread = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("list-add-symbolic")
                .label(gettext("Ajouter un post au fil"))
                .build(),
        )
        .halign(gtk::Align::Start)
        .css_classes(["flat"])
        .build();
    thread_box.append(&add_to_thread);
    content.insert_child_after(&thread_box, Some(&images_area));
    let followups: Rc<RefCell<Vec<Followup>>> = Rc::default();

    // Barre du bas : langue, compteur.
    let language = Rc::new(Cell::new(default_language()));
    let language_menu = gio::Menu::new();
    for (code, name) in LANGUAGES {
        language_menu.append(Some(name), Some(&format!("compose.language::{code}")));
    }
    let language_content = adw::ButtonContent::builder()
        .icon_name("preferences-desktop-locale-symbolic")
        .label(language_name(language.get()))
        .build();
    let language_button = gtk::MenuButton::builder()
        .child(&language_content)
        .menu_model(&language_menu)
        .tooltip_text(gettext("Langue du post"))
        .css_classes(["flat"])
        .build();
    let counter = gtk::Label::builder()
        .label(format!("0 / {MAX_GRAPHEMES}"))
        .hexpand(true)
        .xalign(1.0)
        .css_classes(["dim-label", "numeric"])
        .build();
    let bottom = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .margin_start(6)
        .margin_end(12)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    let add_image = gtk::Button::builder()
        .icon_name("image-x-generic-symbolic")
        .tooltip_text(gettext("Ajouter une image"))
        .css_classes(["flat"])
        .build();
    add_image.update_property(&[gtk::accessible::Property::Label(&gettext(
        "Ajouter une image",
    ))]);
    // Qui peut répondre, et citations : pour un nouveau post seulement (une
    // réponse suit les réglages du début de la discussion).
    let reply_rule = Rc::new(Cell::new(api::ReplyRule::Everyone));
    let no_quotes = Rc::new(Cell::new(false));
    let gate_menu = gio::Menu::new();
    let rules = gio::Menu::new();
    for rule in api::ReplyRule::ALL {
        rules.append(
            Some(&rule.title()),
            Some(&format!("compose.replies::{}", rule.id())),
        );
    }
    gate_menu.append_section(Some(&gettext("Qui peut répondre")), &rules);
    let quotes_section = gio::Menu::new();
    quotes_section.append(
        Some(&gettext("Autoriser les citations")),
        Some("compose.quotes"),
    );
    gate_menu.append_section(None, &quotes_section);
    let gate_content = adw::ButtonContent::builder()
        .icon_name("mail-reply-sender-symbolic")
        .label(api::ReplyRule::Everyone.title())
        .build();
    let gate_button = gtk::MenuButton::builder()
        .child(&gate_content)
        .menu_model(&gate_menu)
        .tooltip_text(gettext("Qui peut répondre et citer"))
        .css_classes(["flat"])
        .visible(!replying)
        .build();
    gate_button.update_property(&[gtk::accessible::Property::Label(&gettext(
        "Qui peut répondre et citer",
    ))]);
    bottom.append(&add_image);
    bottom.append(&language_button);
    bottom.append(&gate_button);
    bottom.append(&counter);

    let view = adw::ToolbarView::builder()
        .content(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&content)
                .build(),
        )
        .build();
    view.add_top_bar(&header);
    view.add_bottom_bar(&bottom);
    dialog.set_child(Some(&view));

    // Action à état pour la langue, que le menu reflète.
    let actions = gio::SimpleActionGroup::new();
    let language_action = gio::SimpleAction::new_stateful(
        "language",
        Some(glib::VariantTy::STRING),
        &language.get().to_variant(),
    );
    {
        let language = language.clone();
        let language_content = language_content.clone();
        language_action.connect_change_state(move |action, value| {
            let Some(code) = value.and_then(|v| v.str()) else {
                return;
            };
            if let Some((code, name)) = LANGUAGES.iter().find(|(c, _)| *c == code) {
                language.set(code);
                language_content.set_label(name);
                action.set_state(&code.to_variant());
            }
        });
    }
    actions.add_action(&language_action);
    let replies_action = gio::SimpleAction::new_stateful(
        "replies",
        Some(glib::VariantTy::STRING),
        &api::ReplyRule::Everyone.id().to_variant(),
    );
    {
        let (reply_rule, gate_content) = (reply_rule.clone(), gate_content.clone());
        replies_action.connect_change_state(move |action, value| {
            if let Some(rule) = value
                .and_then(|v| v.str())
                .and_then(api::ReplyRule::from_id)
            {
                reply_rule.set(rule);
                gate_content.set_label(&rule.title());
                action.set_state(&rule.id().to_variant());
            }
        });
    }
    actions.add_action(&replies_action);
    let quotes_action = gio::SimpleAction::new_stateful("quotes", None, &true.to_variant());
    {
        let no_quotes = no_quotes.clone();
        quotes_action.connect_activate(move |action, _| {
            let allowed = !action.state().and_then(|s| s.get::<bool>()).unwrap_or(true);
            action.set_state(&allowed.to_variant());
            no_quotes.set(!allowed);
        });
    }
    actions.add_action(&quotes_action);
    dialog.insert_action_group("compose", Some(&actions));

    // Compteur et validité, à chaque frappe. Un texte en cours empêche de
    // fermer sans confirmation.
    let busy = Rc::new(Cell::new(false));
    let update: Rc<dyn Fn()> = {
        let (
            publish,
            counter,
            placeholder,
            dialog,
            busy,
            text,
            attachments,
            add_image,
            images_area,
            drafts_button,
            saved_drafts,
            followups,
            add_to_thread,
        ) = (
            publish.clone(),
            counter.clone(),
            placeholder.clone(),
            dialog.clone(),
            busy.clone(),
            text.clone(),
            attachments.clone(),
            add_image.clone(),
            images_area.clone(),
            drafts_button.clone(),
            saved_drafts.clone(),
            followups.clone(),
            add_to_thread.clone(),
        );
        Rc::new(move || {
            let buffer = text.buffer();
            let value = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let images = attachments.borrow().len();
            images_area.set_visible(images > 0);
            add_image.set_sensitive(images < MAX_IMAGES && !busy.get());
            // Une citation suffit, sans texte ni image.
            let result = validate(&value, images + quoting);
            let count = match result {
                Ok(n) | Err(n) => n,
            };
            counter.set_label(&format!("{count} / {MAX_GRAPHEMES}"));
            if count > MAX_GRAPHEMES {
                counter.add_css_class("error");
                counter.remove_css_class("dim-label");
            } else {
                counter.remove_css_class("error");
                counter.add_css_class("dim-label");
            }
            placeholder.set_visible(value.is_empty());
            // Posts suivants : chacun non vide et sous la limite.
            let mut thread_ok = true;
            let mut thread_empty = true;
            for followup in followups.borrow().iter() {
                let value = followup.text();
                let count = grapheme_count(&value);
                followup
                    .counter
                    .set_label(&format!("{count} / {MAX_GRAPHEMES}"));
                let bad = value.trim().is_empty() || count > MAX_GRAPHEMES;
                if count > MAX_GRAPHEMES {
                    followup.counter.add_css_class("error");
                } else {
                    followup.counter.remove_css_class("error");
                }
                thread_ok &= !bad;
                thread_empty &= value.trim().is_empty();
            }
            add_to_thread.set_sensitive(followups.borrow().len() < MAX_FOLLOWUPS && !busy.get());
            drafts_button
                .set_visible(value.is_empty() && can_draft && !saved_drafts.borrow().is_empty());
            publish.set_sensitive(result.is_ok() && thread_ok && !busy.get());
            dialog.set_can_close(value.trim().is_empty() && images == 0 && thread_empty);
        })
    };
    {
        let update = update.clone();
        text.buffer().connect_changed(move |_| update());
    }

    // Ajouter un post au fil (vide, ou avec le texte d'un brouillon).
    let add_followup: Rc<dyn Fn(&str)> = {
        let (thread_box, add_to_thread, followups, update, author) = (
            thread_box.clone(),
            add_to_thread.clone(),
            followups.clone(),
            update.clone(),
            author.clone(),
        );
        Rc::new(move |initial: &str| {
            if followups.borrow().len() >= MAX_FOLLOWUPS {
                return;
            }
            let view = gtk::TextView::builder()
                .wrap_mode(gtk::WrapMode::WordChar)
                .accepts_tab(false)
                .hexpand(true)
                .height_request(60)
                .top_margin(6)
                .css_classes(["compose-text"])
                .build();
            view.buffer().set_text(initial);
            let number = followups.borrow().len() + 2;
            view.update_property(&[gtk::accessible::Property::Label(
                &gettext("Post {} du fil").replace("{}", &number.to_string()),
            )]);
            let counter = gtk::Label::builder()
                .halign(gtk::Align::End)
                .css_classes(["dim-label", "numeric", "caption"])
                .build();
            let remove = gtk::Button::builder()
                .icon_name("window-close-symbolic")
                .tooltip_text(gettext("Retirer ce post du fil"))
                .valign(gtk::Align::Start)
                .css_classes(["flat", "circular"])
                .build();
            remove.update_property(&[gtk::accessible::Property::Label(&gettext(
                "Retirer ce post du fil",
            ))]);
            let column = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .hexpand(true)
                .build();
            column.append(&view);
            column.append(&counter);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.append(
                &adw::Avatar::builder()
                    .size(32)
                    .text(&author)
                    .show_initials(true)
                    .valign(gtk::Align::Start)
                    .margin_start(4)
                    .margin_end(4)
                    .build(),
            );
            row.append(&column);
            row.append(&remove);
            thread_box.insert_child_after(
                &row,
                followups
                    .borrow()
                    .last()
                    .map(|f| f.row.upcast_ref::<gtk::Widget>()),
            );
            if followups.borrow().is_empty() {
                thread_box.reorder_child_after(&row, None::<&gtk::Widget>);
            }
            thread_box.reorder_child_after(&add_to_thread, Some(&row));
            {
                let update = update.clone();
                view.buffer().connect_changed(move |_| update());
            }
            {
                let (followups, thread_box, update) =
                    (followups.clone(), thread_box.clone(), update.clone());
                let weak_row = row.downgrade();
                remove.connect_clicked(move |_| {
                    let Some(row) = weak_row.upgrade() else {
                        return;
                    };
                    followups.borrow_mut().retain(|f| f.row != row);
                    thread_box.remove(&row);
                    update();
                });
            }
            followups.borrow_mut().push(Followup {
                row,
                view: view.clone(),
                counter,
            });
            update();
            if initial.is_empty() {
                view.grab_focus();
            }
        })
    };
    {
        let add_followup = add_followup.clone();
        add_to_thread.connect_clicked(move |_| add_followup(""));
    }

    // Brouillons : la liste, et le choix d'un brouillon qui remplit la boîte.
    {
        let (drafts_list, saved_drafts, text, draft_id, language_action, drafts_button) = (
            drafts_list.clone(),
            saved_drafts.clone(),
            text.clone(),
            draft_id.clone(),
            language_action.clone(),
            drafts_button.clone(),
        );
        let (followups, add_followup) = (followups.clone(), add_followup.clone());
        drafts_list.connect_row_activated(move |_, row| {
            let draft = saved_drafts.borrow().get(row.index() as usize).cloned();
            let Some(draft) = draft else { return };
            drafts_button.popdown();
            draft_id.replace(Some(draft.id.clone()));
            language_action.change_state(&draft.lang.to_variant());
            text.buffer().set_text(&draft.text);
            for followup in followups.borrow_mut().drain(..) {
                if let Some(parent) = followup.row.parent().and_downcast::<gtk::Box>() {
                    parent.remove(&followup.row);
                }
            }
            for more in &draft.more {
                add_followup(more);
            }
            text.grab_focus();
        });
    }
    for draft in saved_drafts.borrow().iter() {
        let first_line = draft.text.lines().next().unwrap_or_default();
        let saved = api::parse_datetime(&draft.saved)
            .map(|t| api::relative_time(t, chrono::Utc::now()))
            .unwrap_or_default();
        let row = adw::ActionRow::builder()
            .title(first_line)
            .title_lines(2)
            .subtitle(&saved)
            .use_markup(false)
            .activatable(true)
            .build();
        let delete = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text(gettext("Supprimer le brouillon"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        delete.update_property(&[gtk::accessible::Property::Label(&gettext(
            "Supprimer le brouillon",
        ))]);
        let (id, saved_drafts, update, list, weak_row) = (
            draft.id.clone(),
            saved_drafts.clone(),
            update.clone(),
            drafts_list.downgrade(),
            row.downgrade(),
        );
        delete.connect_clicked(move |_| {
            if let Err(error) = drafts::remove(&id) {
                glib::g_debug!("tsubame", "brouillon : {error}");
            }
            saved_drafts.borrow_mut().retain(|d| d.id != id);
            if let (Some(list), Some(row)) = (list.upgrade(), weak_row.upgrade()) {
                list.remove(&row);
            }
            update();
        });
        row.add_suffix(&delete);
        drafts_list.append(&row);
    }

    // État de départ : une citation peut se publier sans texte.
    update();

    // Ajouter des images : sélecteur de fichiers du système (portail en
    // Flatpak).
    {
        let (dialog, attachments, tiles, update, error) = (
            dialog.clone(),
            attachments.clone(),
            tiles.clone(),
            update.clone(),
            error.clone(),
        );
        add_image.connect_clicked(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(&gettext("Images")));
            for mime in ["image/png", "image/jpeg", "image/webp"] {
                filter.add_mime_type(mime);
            }
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let chooser = gtk::FileDialog::builder()
                .title(gettext("Ajouter des images"))
                .modal(true)
                .filters(&filters)
                .default_filter(&filter)
                .build();
            let window = dialog.root().and_downcast::<gtk::Window>();
            let (attachments, tiles, update, error) = (
                attachments.clone(),
                tiles.clone(),
                update.clone(),
                error.clone(),
            );
            chooser.open_multiple(window.as_ref(), gio::Cancellable::NONE, move |result| {
                let Ok(files) = result else { return };
                let files: Vec<gio::File> =
                    files.iter::<gio::File>().filter_map(Result::ok).collect();
                glib::spawn_future_local(async move {
                    for file in files {
                        if attachments.borrow().len() >= MAX_IMAGES {
                            break;
                        }
                        let loaded = file
                            .load_bytes_future()
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|(bytes, _)| {
                                gdk::Texture::from_bytes(&bytes)
                                    .map(|texture| (bytes, texture))
                                    .map_err(|e| e.to_string())
                            });
                        match loaded {
                            Ok((bytes, texture)) => {
                                attach(&attachments, &tiles, &update, bytes, texture);
                            }
                            Err(_) => {
                                error
                                    .set_label(&gettext("Ce fichier n'est pas une image lisible."));
                                error.set_visible(true);
                            }
                        }
                    }
                    update();
                });
            });
        });
    }

    // Fermer avec un texte en cours : demander confirmation.
    // Fermer avec un texte en cours : demander confirmation, et proposer
    // de garder un brouillon pour un nouveau post.
    {
        let (text, language, draft_id, attachments, error) = (
            text.clone(),
            language.clone(),
            draft_id.clone(),
            attachments.clone(),
            error.clone(),
        );
        let followups = followups.clone();
        dialog.connect_close_attempt(move |dialog| {
            let has_images = !attachments.borrow().is_empty();
            let body = if can_draft && has_images {
                gettext("Le brouillon garde le texte, mais pas les images.")
            } else if can_draft {
                gettext("Vous pouvez garder le texte en brouillon.")
            } else {
                gettext("Le texte saisi sera perdu.")
            };
            let confirm = adw::AlertDialog::new(
                Some(&if can_draft {
                    gettext("Garder ce post en brouillon ?")
                } else {
                    gettext("Abandonner ce post ?")
                }),
                Some(&body),
            );
            confirm.add_responses(&[
                ("continue", &gettext("Continuer la rédaction")),
                ("discard", &gettext("Abandonner")),
            ]);
            if can_draft {
                confirm.add_response("save", &gettext("Garder en brouillon"));
                confirm.set_response_appearance("save", adw::ResponseAppearance::Suggested);
                confirm.set_default_response(Some("save"));
            } else {
                confirm.set_default_response(Some("continue"));
            }
            confirm.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
            confirm.set_close_response("continue");
            let weak = dialog.downgrade();
            confirm.connect_response(Some("discard"), move |_, _| {
                if let Some(dialog) = weak.upgrade() {
                    dialog.force_close();
                }
            });
            let weak = dialog.downgrade();
            let (text, language, draft_id, error) = (
                text.clone(),
                language.clone(),
                draft_id.clone(),
                error.clone(),
            );
            let more: Vec<String> = followups
                .borrow()
                .iter()
                .map(Followup::text)
                .filter(|t| !t.trim().is_empty())
                .collect();
            confirm.connect_response(Some("save"), move |_, _| {
                let Some(dialog) = weak.upgrade() else { return };
                let buffer = text.buffer();
                let now = chrono::Utc::now().to_rfc3339();
                let draft = Draft {
                    id: draft_id.borrow().clone().unwrap_or_else(|| now.clone()),
                    text: buffer
                        .text(&buffer.start_iter(), &buffer.end_iter(), false)
                        .to_string(),
                    more: more.clone(),
                    lang: language.get().to_owned(),
                    saved: now,
                };
                match drafts::put(draft) {
                    Ok(()) => dialog.force_close(),
                    Err(failure) => {
                        glib::g_debug!("tsubame", "brouillon : {failure}");
                        error.set_label(&gettext("Le brouillon n'a pas pu être enregistré."));
                        error.set_visible(true);
                    }
                }
            });
            confirm.present(Some(dialog));
        });
    }
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }

    // Publier.
    let on_published = Rc::new(on_published);
    let parent_ref = reply_to.as_ref().and_then(|post| {
        Some(StrongRef {
            uri: post.uri.clone()?,
            cid: post.cid.clone()?,
        })
    });
    let quote_ref = quote.as_ref().and_then(|post| {
        Some(StrongRef {
            uri: post.uri.clone()?,
            cid: post.cid.clone()?,
        })
    });
    let reply = parent_ref.clone().map(|parent| {
        let root = reply_to
            .as_ref()
            .and_then(|post| post.reply_root.clone())
            .unwrap_or_else(|| parent.clone());
        (root, parent)
    });
    let do_publish = {
        let (dialog, text, publish, error, toasts, busy, language, attachments, draft_id) = (
            dialog.clone(),
            text.clone(),
            publish.clone(),
            error.clone(),
            toasts.clone(),
            busy.clone(),
            language.clone(),
            attachments.clone(),
            draft_id.clone(),
        );
        let followups = followups.clone();
        move || {
            let more: Vec<String> = followups
                .borrow()
                .iter()
                .map(|f| f.text().trim().to_owned())
                .collect();
            if more
                .iter()
                .any(|t| t.is_empty() || grapheme_count(t) > MAX_GRAPHEMES)
            {
                return;
            }
            let buffer = text.buffer();
            let value = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .trim()
                .to_owned();
            let images: Vec<(Vec<u8>, String)> = attachments
                .borrow()
                .iter()
                .map(|a| (a.bytes.to_vec(), a.alt.borrow().trim().to_owned()))
                .collect();
            if validate(&value, images.len() + quoting).is_err() || busy.get() {
                return;
            }
            let Some(session) = session.clone() else {
                // Mode démonstration : rien n'est envoyé.
                toasts.add_toast(adw::Toast::new(&gettext(
                    "Mode démonstration : rien n'est publié.",
                )));
                dialog.force_close();
                return;
            };
            busy.set(true);
            publish.set_sensitive(false);
            publish.set_label(&gettext("Publication…"));
            text.set_editable(false);
            error.set_visible(false);
            let post = NewPost {
                text: value,
                langs: vec![language.get().to_owned()],
                reply: reply.clone(),
                images: Vec::new(),
                quote: quote_ref.clone(),
                reply_rule: reply_rule.get(),
                no_quotes: no_quotes.get(),
            };
            let task = runtime().spawn(async move {
                // Images réduites et réencodées hors de la boucle GTK.
                let mut post = post;
                for (bytes, alt) in images {
                    let prepared = tokio::task::spawn_blocking(move || photo::prepare(&bytes))
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r)
                        .map_err(|e| {
                            api::ApiError::new(
                                gettext("Une image n'a pas pu être préparée : {}")
                                    .replace("{}", &e),
                            )
                        })?;
                    post.images.push(NewImage { prepared, alt });
                }
                // En boîte : le futur de publication est profond.
                let langs = post.langs.clone();
                let root = post.reply.as_ref().map(|(root, _)| root.clone());
                let published = Box::pin(api::publish(session.clone(), post)).await?;
                // Posts suivants : chacun répond au précédent. Un échec
                // arrête le fil, sans rien republier.
                let root = root.unwrap_or_else(|| published.post.clone());
                let mut parent = published.post.clone();
                let mut failure = None;
                let mut done = 0;
                for text in more {
                    let next = NewPost {
                        text,
                        langs: langs.clone(),
                        reply: Some((root.clone(), parent.clone())),
                        images: Vec::new(),
                        quote: None,
                        reply_rule: api::ReplyRule::Everyone,
                        no_quotes: false,
                    };
                    match Box::pin(api::publish(session.clone(), next)).await {
                        Ok(next) => {
                            parent = next.post;
                            done += 1;
                        }
                        Err(error) => {
                            failure = Some(error);
                            break;
                        }
                    }
                }
                Ok((published, done, failure))
            });
            let total = followups.borrow().len();
            let (dialog, text, publish, error, toasts, busy, on_published, parent_uri) = (
                dialog.clone(),
                text.clone(),
                publish.clone(),
                error.clone(),
                toasts.clone(),
                busy.clone(),
                on_published.clone(),
                parent_ref.as_ref().map(|p| p.uri.clone()),
            );
            let draft_id = draft_id.clone();
            glib::spawn_future_local(async move {
                let result = task.await.unwrap_or_else(|_| {
                    Err(api::ApiError::new(gettext(
                        "La publication a été interrompue.",
                    )))
                });
                busy.set(false);
                match result {
                    Ok((published, done, failure)) => {
                        toasts.add_toast(adw::Toast::new(&if let Some(failure) = failure {
                            glib::g_debug!("tsubame", "fil : {}", failure.message);
                            gettext("Fil publié en partie : {} posts sur {}.")
                                .replacen("{}", &(done + 1).to_string(), 1)
                                .replacen("{}", &(total + 1).to_string(), 1)
                        } else if published.gates_failed {
                            gettext(
                                "Post publié, mais ses réglages de réponse et de citation \
                                 n'ont pas été enregistrés.",
                            )
                        } else if total > 0 {
                            gettext("Fil publié.")
                        } else if parent_uri.is_some() {
                            gettext("Réponse publiée.")
                        } else {
                            gettext("Post publié.")
                        }));
                        if let Some(id) = draft_id.borrow().as_deref()
                            && let Err(error) = drafts::remove(id)
                        {
                            glib::g_debug!("tsubame", "brouillon : {error}");
                        }
                        dialog.force_close();
                        on_published(parent_uri);
                    }
                    Err(failure) => {
                        error.set_label(&failure.message);
                        error.set_visible(true);
                        publish.set_label(&gettext("Publier"));
                        publish.set_sensitive(true);
                        text.set_editable(true);
                    }
                }
            });
        }
    };
    let do_publish = Rc::new(do_publish);
    {
        let do_publish = do_publish.clone();
        publish.connect_clicked(move |_| do_publish());
    }
    // Ctrl+Entrée publie.
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.add_shortcut(gtk::Shortcut::new(
        gtk::ShortcutTrigger::parse_string("<Control>Return"),
        Some(gtk::CallbackAction::new(move |_, _| {
            do_publish();
            glib::Propagation::Stop
        })),
    ));
    dialog.add_controller(shortcuts);

    dialog.present(Some(parent));
    text.grab_focus();
}

/// Côté des vignettes, en pixels.
const THUMBNAIL: i32 = 96;
/// Images jointes à un post, au plus.
const MAX_IMAGES: usize = 4;
/// Longueur maximale d'un texte alternatif sur Bluesky.
const MAX_ALT: usize = 2000;

/// Image jointe : le fichier tel quel (préparé à la publication), son
/// aperçu et son texte alternatif.
struct Attachment {
    bytes: glib::Bytes,
    texture: gdk::Texture,
    alt: RefCell<String>,
    tile: gtk::Overlay,
    alt_button: gtk::Button,
}

impl Attachment {
    /// « + ALT » ou « ALT ✓ », comme dans la maquette, et les noms
    /// accessibles qui vont avec.
    fn update_alt(&self) {
        let has_alt = !self.alt.borrow().trim().is_empty();
        self.alt_button.set_label(&if has_alt {
            gettext("ALT ✓")
        } else {
            gettext("+ ALT")
        });
        let (button, image) = if has_alt {
            (
                gettext("Modifier le texte alternatif"),
                self.alt.borrow().clone(),
            )
        } else {
            (
                gettext("Ajouter un texte alternatif"),
                gettext("Image sans texte alternatif"),
            )
        };
        self.alt_button
            .update_property(&[gtk::accessible::Property::Label(&button)]);
        self.tile
            .update_property(&[gtk::accessible::Property::Label(&image)]);
    }
}

/// Vignette carrée de `THUMBNAIL` pixels. `GtkPicture` demande la taille
/// réelle de l'image ; deux `AdwClamp` la bornent en largeur et en hauteur.
fn thumbnail_frame(picture: &gtk::Picture) -> adw::Clamp {
    let vertical = adw::Clamp::builder()
        .orientation(gtk::Orientation::Vertical)
        .maximum_size(THUMBNAIL)
        .tightening_threshold(THUMBNAIL)
        .child(picture)
        .build();
    adw::Clamp::builder()
        .maximum_size(THUMBNAIL)
        .tightening_threshold(THUMBNAIL)
        .halign(gtk::Align::Start)
        .child(&vertical)
        .build()
}

/// Ajoute une vignette : l'image, « Retirer l'image » et le bouton du
/// texte alternatif.
fn attach(
    attachments: &Rc<RefCell<Vec<Rc<Attachment>>>>,
    tiles: &gtk::Box,
    update: &Rc<dyn Fn()>,
    bytes: glib::Bytes,
    texture: gdk::Texture,
) {
    let picture = gtk::Picture::builder()
        .paintable(&texture)
        .content_fit(gtk::ContentFit::Cover)
        .width_request(THUMBNAIL)
        .height_request(THUMBNAIL)
        .css_classes(["compose-thumb"])
        .build();
    let remove = gtk::Button::builder()
        .icon_name("window-close-symbolic")
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .tooltip_text(gettext("Retirer l'image"))
        .css_classes(["circular", "osd", "compose-thumb-button"])
        .build();
    remove.update_property(&[gtk::accessible::Property::Label(&gettext(
        "Retirer l'image",
    ))]);
    let alt_button = gtk::Button::builder()
        .halign(gtk::Align::Start)
        .valign(gtk::Align::End)
        .css_classes(["osd", "compose-thumb-button", "compose-alt"])
        .build();
    let tile = gtk::Overlay::builder()
        .child(&thumbnail_frame(&picture))
        .overflow(gtk::Overflow::Hidden)
        .accessible_role(gtk::AccessibleRole::Group)
        .css_classes(["compose-tile"])
        .build();
    tile.add_overlay(&remove);
    tile.add_overlay(&alt_button);
    tiles.append(&tile);

    let attachment = Rc::new(Attachment {
        bytes,
        texture,
        alt: RefCell::default(),
        tile: tile.clone(),
        alt_button: alt_button.clone(),
    });
    attachment.update_alt();
    attachments.borrow_mut().push(attachment.clone());

    {
        let (attachments, tiles, update, attachment) = (
            attachments.clone(),
            tiles.clone(),
            update.clone(),
            Rc::downgrade(&attachment),
        );
        remove.connect_clicked(move |_| {
            let Some(attachment) = attachment.upgrade() else {
                return;
            };
            tiles.remove(&attachment.tile);
            attachments
                .borrow_mut()
                .retain(|a| !Rc::ptr_eq(a, &attachment));
            update();
        });
    }
    let weak = Rc::downgrade(&attachment);
    alt_button.connect_clicked(move |button| {
        if let Some(attachment) = weak.upgrade() {
            edit_alt(button, &attachment);
        }
    });
}

/// Boîte du texte alternatif : l'image en grand, le texte, un compteur.
fn edit_alt(parent: &gtk::Button, attachment: &Rc<Attachment>) {
    let dialog = adw::Dialog::builder()
        .title(gettext("Texte alternatif"))
        .content_width(480)
        .build();
    let cancel = gtk::Button::with_label(&gettext("Annuler"));
    let save = gtk::Button::builder()
        .label(gettext("Enregistrer"))
        .css_classes(["suggested-action"])
        .build();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&save);

    let preview = gtk::Picture::builder()
        .paintable(&attachment.texture)
        .content_fit(gtk::ContentFit::Contain)
        .height_request(200)
        .build();
    preview.update_property(&[gtk::accessible::Property::Label(&gettext(
        "Image à décrire",
    ))]);
    let text = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .height_request(96)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .css_classes(["card", "compose-alt-text"])
        .build();
    text.buffer().set_text(&attachment.alt.borrow());
    text.update_property(&[gtk::accessible::Property::Label(&gettext(
        "Texte alternatif",
    ))]);
    let help = gtk::Label::builder()
        .label(gettext(
            "Décrivez ce que montre l'image pour les personnes qui ne la voient pas.",
        ))
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label", "caption"])
        .build();
    let counter = gtk::Label::builder()
        .xalign(1.0)
        .css_classes(["dim-label", "numeric", "caption"])
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&preview);
    content.append(&text);
    content.append(&help);
    content.append(&counter);
    let view = adw::ToolbarView::builder().content(&content).build();
    view.add_top_bar(&header);
    dialog.set_child(Some(&view));

    let refresh = {
        let (counter, save) = (counter.clone(), save.clone());
        move |buffer: &gtk::TextBuffer| {
            let value = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let count = grapheme_count(&value);
            counter.set_label(&format!("{count} / {MAX_ALT}"));
            save.set_sensitive(count <= MAX_ALT);
            if count > MAX_ALT {
                counter.add_css_class("error");
            } else {
                counter.remove_css_class("error");
            }
        }
    };
    refresh(&text.buffer());
    text.buffer().connect_changed(refresh);
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    {
        let (dialog, text, attachment) = (dialog.clone(), text.clone(), attachment.clone());
        save.connect_clicked(move |_| {
            let buffer = text.buffer();
            attachment.alt.replace(
                buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .to_string(),
            );
            attachment.update_alt();
            dialog.close();
        });
    }
    dialog.present(Some(parent));
    text.grab_focus();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_or_too_long_posts_cannot_be_published() {
        assert_eq!(validate("Bonjour", 0), Ok(7));
        assert_eq!(validate("   \n ", 0), Err(5));
        // Un post fait seulement d'images peut être publié.
        assert_eq!(validate("", 1), Ok(0));
        assert_eq!(validate(&"é".repeat(300), 0), Ok(300));
        assert_eq!(validate(&"é".repeat(301), 2), Err(301));
    }

    #[test]
    fn languages_have_names() {
        assert_eq!(language_name("de"), "Deutsch");
        assert!(
            LANGUAGES
                .iter()
                .any(|(code, _)| *code == default_language())
        );
    }
}
