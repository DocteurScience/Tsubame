//! Boîte de rédaction : nouveau post ou réponse (maquette
//! `docs/mockups/source/Compose.dc.html`).
//!
//! Pour l'instant : texte, langue, compteur de caractères, réponse. Les
//! liens, mots-dièse et mentions deviennent des facettes à la publication.
//! À venir : images, vidéo, étiquettes, réglages de réponse, brouillons.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};

use crate::api::{self, NewPost};
use crate::auth::Session;
use crate::data::{Post, StrongRef};
use crate::richtext::{MAX_GRAPHEMES, grapheme_count};
use crate::runtime::runtime;

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

/// Ce qui empêche de publier, ou `None`.
pub fn validate(text: &str) -> Result<usize, usize> {
    let count = grapheme_count(text);
    if text.trim().is_empty() || count > MAX_GRAPHEMES {
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
    on_published: impl Fn(Option<String>) + 'static,
) {
    let replying = reply_to.is_some();
    let dialog = adw::Dialog::builder()
        .title(if replying {
            gettext("Répondre")
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

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    // Post auquel on répond, en rappel.
    if let Some(post) = &reply_to {
        let quoted = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .css_classes(["card", "compose-quote"])
            .build();
        quoted.append(
            &gtk::Label::builder()
                .label(gettext("En réponse à {}").replace("{}", &post.name))
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
    bottom.append(&language_button);
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
    dialog.insert_action_group("compose", Some(&actions));

    // Compteur et validité, à chaque frappe. Un texte en cours empêche de
    // fermer sans confirmation.
    let busy = Rc::new(Cell::new(false));
    let update = {
        let (publish, counter, placeholder, dialog, busy) = (
            publish.clone(),
            counter.clone(),
            placeholder.clone(),
            dialog.clone(),
            busy.clone(),
        );
        move |buffer: &gtk::TextBuffer| {
            let value = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let result = validate(&value);
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
            publish.set_sensitive(result.is_ok() && !busy.get());
            dialog.set_can_close(value.trim().is_empty());
        }
    };
    text.buffer().connect_changed(update.clone());

    // Fermer avec un texte en cours : demander confirmation.
    dialog.connect_close_attempt(|dialog| {
        let confirm = adw::AlertDialog::new(
            Some(&gettext("Abandonner ce post ?")),
            Some(&gettext("Le texte saisi sera perdu.")),
        );
        confirm.add_responses(&[
            ("continue", &gettext("Continuer la rédaction")),
            ("discard", &gettext("Abandonner")),
        ]);
        confirm.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        confirm.set_default_response(Some("continue"));
        confirm.set_close_response("continue");
        let weak = dialog.downgrade();
        confirm.connect_response(Some("discard"), move |_, _| {
            if let Some(dialog) = weak.upgrade() {
                dialog.force_close();
            }
        });
        confirm.present(Some(dialog));
    });
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
    let reply = parent_ref.clone().map(|parent| {
        let root = reply_to
            .as_ref()
            .and_then(|post| post.reply_root.clone())
            .unwrap_or_else(|| parent.clone());
        (root, parent)
    });
    let do_publish = {
        let (dialog, text, publish, error, toasts, busy, language) = (
            dialog.clone(),
            text.clone(),
            publish.clone(),
            error.clone(),
            toasts.clone(),
            busy.clone(),
            language.clone(),
        );
        move || {
            let buffer = text.buffer();
            let value = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .trim()
                .to_owned();
            if validate(&value).is_err() || busy.get() {
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
            };
            let task = runtime().spawn(api::publish(session, post));
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
            glib::spawn_future_local(async move {
                let result = task.await.unwrap_or_else(|_| {
                    Err(api::ApiError::new(gettext(
                        "La publication a été interrompue.",
                    )))
                });
                busy.set(false);
                match result {
                    Ok(_) => {
                        toasts.add_toast(adw::Toast::new(&if parent_uri.is_some() {
                            gettext("Réponse publiée.")
                        } else {
                            gettext("Post publié.")
                        }));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_or_too_long_posts_cannot_be_published() {
        assert_eq!(validate("Bonjour"), Ok(7));
        assert_eq!(validate("   \n "), Err(5));
        assert_eq!(validate(&"é".repeat(300)), Ok(300));
        assert_eq!(validate(&"é".repeat(301)), Err(301));
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
