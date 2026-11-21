//! Vue d'une discussion : le post ouvert en grand, puis ses réponses.
//!
//! La fenêtre en crée deux : une dans le volet à côté des colonnes, une dans
//! la page empilée (paliers étroits, volet masqué). Les deux montrent le même
//! post.

use adw::prelude::*;
use gettextrs::{gettext, ngettext};

use crate::card;
use crate::data::{Post, Reply};

pub struct ThreadView {
    pub root: adw::ToolbarView,
    header: adw::HeaderBar,
    selector: gtk::Box,
    stack: gtk::Stack,
    content: gtk::Box,
    replies: gtk::Box,
}

/// Sélecteur de position du volet de lecture : trois boutons liés à l'action
/// à état `win.pane-position`, qui coche celui de la position courante.
///
/// À remplacer par `AdwToggleGroup` quand la version minimale de libadwaita
/// passera à 1.7.
fn pane_selector() -> gtk::Box {
    let selector = gtk::Box::builder()
        .accessible_role(gtk::AccessibleRole::Group)
        .css_classes(["linked"])
        .build();
    selector.update_property(&[gtk::accessible::Property::Label(&gettext(
        "Disposition du volet de lecture",
    ))]);
    for (position, label) in [
        ("right", gettext("Volet à droite")),
        ("below", gettext("Volet en dessous")),
        ("hidden", gettext("Volet masqué")),
    ] {
        let button = gtk::ToggleButton::builder()
            .icon_name(format!("tsubame-pane-{position}-symbolic"))
            .action_name("win.pane-position")
            .action_target(&position.to_variant())
            .tooltip_text(&label)
            .build();
        button.update_property(&[gtk::accessible::Property::Label(&label)]);
        selector.append(&button);
    }
    selector
}

impl ThreadView {
    pub fn new() -> Self {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(
                &adw::Clamp::builder()
                    .maximum_size(720)
                    .child(&content)
                    .build(),
            )
            .build();
        let stack = gtk::Stack::new();
        stack.add_named(
            &adw::StatusPage::builder()
                .icon_name(format!("{}-symbolic", crate::APP_ID))
                .title(gettext("Aucun post ouvert"))
                .description(gettext("Choisissez un post pour afficher la discussion."))
                .build(),
            Some("empty"),
        );
        stack.add_named(&scroll, Some("thread"));

        let selector = pane_selector();
        let header = adw::HeaderBar::builder()
            .title_widget(&adw::WindowTitle::new(&gettext("Discussion"), ""))
            .build();
        header.pack_start(&selector);
        let root = adw::ToolbarView::builder().content(&stack).build();
        root.add_top_bar(&header);
        Self {
            root,
            header,
            selector,
            stack,
            content,
            replies: gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(12)
                .build(),
        }
    }

    /// Le sélecteur n'a de sens que sur grand écran.
    pub fn set_selector_visible(&self, visible: bool) {
        self.selector.set_visible(visible);
    }

    /// Le volet porte les boutons de la fenêtre seulement quand il est à
    /// droite ; en dessous, ils sont sur la dernière colonne.
    pub fn set_window_controls(&self, visible: bool) {
        self.header.set_show_end_title_buttons(visible);
    }

    /// Affiche un post. Les faux posts ont leurs réponses avec eux ; pour
    /// un post réel, `set_replies` les ajoute une fois chargées.
    pub fn show(&self, post: Option<&Post>) {
        let Some(post) = post else {
            self.stack.set_visible_child_name("empty");
            return;
        };
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        self.content.append(&card::post_detail(post));
        self.content.append(&self.replies);
        if post.uri.is_some() {
            self.set_replies(None);
        } else {
            self.set_replies(Some(Ok(&post.thread)));
        }
        self.stack.set_visible_child_name("thread");
    }

    /// Réponses : `None` pendant le chargement, sinon la liste ou l'erreur.
    pub fn set_replies(&self, replies: Option<Result<&[Reply], &str>>) {
        while let Some(child) = self.replies.first_child() {
            self.replies.remove(&child);
        }
        let heading = |text: String| {
            gtk::Label::builder()
                .label(text)
                .xalign(0.0)
                .margin_top(12)
                .wrap(true)
                .css_classes(["heading"])
                .build()
        };
        match replies {
            None => {
                let spinner = gtk::Spinner::builder()
                    .spinning(true)
                    .halign(gtk::Align::Center)
                    .margin_top(12)
                    .build();
                spinner.update_property(&[gtk::accessible::Property::Label(&gettext(
                    "Chargement des réponses",
                ))]);
                self.replies.append(&spinner);
            }
            Some(Err(message)) => {
                let label = heading(message.to_owned());
                label.remove_css_class("heading");
                label.add_css_class("dim-label");
                self.replies.append(&label);
            }
            Some(Ok([])) => {}
            Some(Ok(replies)) => {
                let count = replies.len() as u32;
                self.replies.append(&heading(
                    ngettext("{} réponse", "{} réponses", count).replace("{}", &count.to_string()),
                ));
                for reply in replies {
                    self.replies.append(&card::reply_card(reply));
                }
            }
        }
    }
}
