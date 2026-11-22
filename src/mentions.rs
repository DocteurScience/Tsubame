//! Suggestions de mentions : « @dé » dans la boîte de rédaction propose des
//! comptes (`searchActorsTypeahead`). Flèches pour choisir, Entrée ou
//! Tabulation pour compléter, Échap pour fermer.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gdk, glib};

use crate::api::{self, ApiError};
use crate::auth::Session;
use crate::data::{self, Actor};
use crate::images;
use crate::runtime::runtime;

/// Suggestions affichées, au plus.
const MAX_SUGGESTIONS: usize = 6;

fn is_handle_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '.' || c == '-'
}

/// Mention en cours de saisie à la fin de `before` (le texte avant le
/// curseur) : la position de son « @ », en caractères, et ce qui suit.
/// Un « @ » collé à un mot (adresse e-mail) n'en est pas une.
pub fn mention_before(before: &str) -> Option<(usize, String)> {
    let at = before.rfind('@')?;
    let prefix = &before[at + 1..];
    if prefix.is_empty() || !prefix.chars().all(is_handle_char) {
        return None;
    }
    if before[..at]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || c == '@')
    {
        return None;
    }
    Some((before[..at].chars().count(), prefix.to_owned()))
}

/// Comptes de démonstration dont l'identifiant ou le nom commence par
/// `prefix`.
fn demo_matches(prefix: &str) -> Vec<Actor> {
    let prefix = prefix.to_lowercase();
    data::demo_actors()
        .into_iter()
        .filter(|actor| {
            actor.handle.to_lowercase().starts_with(&prefix)
                || actor
                    .name
                    .to_lowercase()
                    .split_whitespace()
                    .any(|word| word.starts_with(&prefix))
        })
        .collect()
}

fn suggestion_row(actor: &Actor) -> gtk::ListBoxRow {
    let avatar = adw::Avatar::builder()
        .size(28)
        .text(&actor.name)
        .show_initials(true)
        .build();
    if let Some(url) = &actor.avatar {
        let weak = avatar.downgrade();
        images::load(url, move |texture| {
            if let Some(avatar) = weak.upgrade() {
                avatar.set_custom_image(Some(texture));
            }
        });
    }
    let labels = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .build();
    for (text, classes) in [
        (actor.name.as_str(), &["heading"][..]),
        (&format!("@{}", actor.handle), &["dim-label", "caption"][..]),
    ] {
        labels.append(
            &gtk::Label::builder()
                .label(text)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .max_width_chars(32)
                .css_classes(classes.to_vec())
                .build(),
        );
    }
    let content = gtk::Box::builder()
        .spacing(8)
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(8)
        .build();
    content.append(&avatar);
    content.append(&labels);
    let row = gtk::ListBoxRow::builder().child(&content).build();
    row.update_property(&[gtk::accessible::Property::Label(&format!(
        "{} @{}",
        actor.name, actor.handle
    ))]);
    row
}

struct Completion {
    /// Référence faible : la zone de texte garde la complétion.
    view: glib::WeakRef<gtk::TextView>,
    popover: gtk::Popover,
    list: gtk::ListBox,
    handles: RefCell<Vec<String>>,
    /// Numéro de la dernière saisie : une réponse plus ancienne est ignorée.
    generation: Cell<u64>,
    session: Option<Arc<Session>>,
}

impl Completion {
    /// Texte avant le curseur, et la position du curseur.
    fn before_cursor(&self) -> Option<(String, gtk::TextIter)> {
        let buffer = self.view.upgrade()?.buffer();
        let cursor = buffer.iter_at_mark(&buffer.get_insert());
        let mut line_start = cursor;
        line_start.set_line_offset(0);
        Some((buffer.text(&line_start, &cursor, false).to_string(), cursor))
    }

    fn changed(self: &Rc<Self>) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let before = self
            .before_cursor()
            .map(|(text, _)| text)
            .unwrap_or_default();
        let Some((_, prefix)) = mention_before(&before) else {
            self.popover.popdown();
            return;
        };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            // Attendre une courte pause dans la frappe.
            glib::timeout_future(Duration::from_millis(150)).await;
            let Some(this) = weak.upgrade() else { return };
            if this.generation.get() != generation {
                return;
            }
            let actors = match this.session.clone() {
                None => demo_matches(&prefix),
                Some(session) => {
                    let task = runtime().spawn(api::typeahead(session, prefix));
                    let result: Result<Vec<Actor>, ApiError> = task
                        .await
                        .unwrap_or_else(|_| Err(ApiError::new(String::new())));
                    result.unwrap_or_default()
                }
            };
            if this.generation.get() != generation {
                return;
            }
            this.show(actors);
        });
    }

    fn show(&self, actors: Vec<Actor>) {
        self.list.remove_all();
        let actors: Vec<Actor> = actors.into_iter().take(MAX_SUGGESTIONS).collect();
        if actors.is_empty() {
            self.popover.popdown();
            return;
        }
        for actor in &actors {
            self.list.append(&suggestion_row(actor));
        }
        self.handles
            .replace(actors.into_iter().map(|a| a.handle).collect());
        self.list.select_row(self.list.row_at_index(0).as_ref());
        // Sous le curseur.
        let (Some(view), Some((_, cursor))) = (self.view.upgrade(), self.before_cursor()) else {
            return;
        };
        let location = view.iter_location(&cursor);
        let (x, y) =
            view.buffer_to_window_coords(gtk::TextWindowType::Widget, location.x(), location.y());
        self.popover
            .set_pointing_to(Some(&gdk::Rectangle::new(x, y, 1, location.height())));
        self.popover.popup();
    }

    fn move_selection(&self, delta: i32) {
        let count = self.handles.borrow().len() as i32;
        if count == 0 {
            return;
        }
        let current = self.list.selected_row().map(|r| r.index()).unwrap_or(0);
        let next = (current + delta).rem_euclid(count);
        self.list.select_row(self.list.row_at_index(next).as_ref());
    }

    /// Remplace la mention en cours par l'identifiant choisi.
    fn complete(&self, index: i32) {
        let Some(handle) = self.handles.borrow().get(index as usize).cloned() else {
            return;
        };
        let (Some(view), Some((before, mut cursor))) = (self.view.upgrade(), self.before_cursor())
        else {
            return;
        };
        let Some((at, _)) = mention_before(&before) else {
            return;
        };
        let buffer = view.buffer();
        let mut start = cursor;
        start.set_line_offset(at as i32);
        buffer.delete(&mut start, &mut cursor);
        buffer.insert(&mut start, &format!("@{handle} "));
        // Le changement de texte a relancé la recherche : l'annuler.
        self.generation.set(self.generation.get() + 1);
        self.popover.popdown();
        view.grab_focus();
    }
}

/// Ajoute les suggestions de mentions à une zone de texte.
pub fn attach(view: &gtk::TextView, session: Option<Arc<Session>>) {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(["navigation-sidebar"])
        .build();
    list.update_property(&[gtk::accessible::Property::Label(&gettext(
        "Comptes à mentionner",
    ))]);
    let popover = gtk::Popover::builder()
        .child(&list)
        .autohide(false)
        .has_arrow(false)
        .can_focus(false)
        .position(gtk::PositionType::Bottom)
        .build();
    popover.set_parent(view);
    let completion = Rc::new(Completion {
        view: view.downgrade(),
        popover: popover.clone(),
        list: list.clone(),
        handles: RefCell::default(),
        generation: Cell::new(0),
        session,
    });

    let weak = Rc::downgrade(&completion);
    view.buffer().connect_changed(move |_| {
        if let Some(completion) = weak.upgrade() {
            completion.changed();
        }
    });
    let weak = Rc::downgrade(&completion);
    list.connect_row_activated(move |_, row| {
        if let Some(completion) = weak.upgrade() {
            completion.complete(row.index());
        }
    });

    // Clavier : seulement quand la liste est ouverte.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(&completion);
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(completion) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if !completion.popover.is_visible() {
            return glib::Propagation::Proceed;
        }
        match key {
            gdk::Key::Down => completion.move_selection(1),
            gdk::Key::Up => completion.move_selection(-1),
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::Tab | gdk::Key::ISO_Left_Tab => {
                let index = completion
                    .list
                    .selected_row()
                    .map(|r| r.index())
                    .unwrap_or(0);
                completion.complete(index);
            }
            gdk::Key::Escape => completion.popover.popdown(),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    view.add_controller(keys);
    let weak = Rc::downgrade(&completion);
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(move |_| {
        if let Some(completion) = weak.upgrade() {
            completion.popover.popdown();
        }
    });
    view.add_controller(focus);

    // La zone de texte garde la complétion ; le popover s'en détache avec
    // elle.
    view.connect_destroy(move |_| {
        completion.popover.unparent();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_are_found_before_the_cursor() {
        assert_eq!(mention_before("Merci @luc"), Some((6, "luc".into())));
        assert_eq!(
            mention_before("@lucie.example"),
            Some((0, "lucie.example".into()))
        );
        assert_eq!(mention_before("Bravo (@ay"), Some((7, "ay".into())));
        assert_eq!(mention_before("é @na"), Some((2, "na".into())));
        // Rien après « @ », espace après, adresse e-mail : pas de mention.
        assert_eq!(mention_before("Merci @"), None);
        assert_eq!(mention_before("Merci @lucie "), None);
        assert_eq!(mention_before("moi@exemple.fr"), None);
        assert_eq!(mention_before("Sans mention"), None);
    }

    #[test]
    fn demo_accounts_match_handle_or_name() {
        assert!(
            demo_matches("nad")
                .iter()
                .any(|a| a.handle.starts_with("nadia"))
        );
        assert!(
            demo_matches("saf")
                .iter()
                .any(|a| a.handle.starts_with("nadia"))
        );
        assert!(demo_matches("zzz").is_empty());
    }
}
