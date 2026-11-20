//! Vue d'une discussion : le post ouvert en grand, puis ses réponses.
//!
//! La fenêtre en crée deux : une dans le volet à droite des colonnes, une dans
//! la page empilée des paliers étroits. Les deux montrent le même post.

use adw::prelude::*;
use gettextrs::{gettext, ngettext};

use crate::card;
use crate::data::Post;

pub struct ThreadView {
    pub root: adw::ToolbarView,
    stack: gtk::Stack,
    content: gtk::Box,
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

        let root = adw::ToolbarView::builder().content(&stack).build();
        root.add_top_bar(
            &adw::HeaderBar::builder()
                .title_widget(&adw::WindowTitle::new(&gettext("Discussion"), ""))
                .build(),
        );
        Self {
            root,
            stack,
            content,
        }
    }

    pub fn show(&self, post: Option<&Post>) {
        let Some(post) = post else {
            self.stack.set_visible_child_name("empty");
            return;
        };
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        self.content.append(&card::post_detail(post));
        if !post.thread.is_empty() {
            let count = post.thread.len() as u32;
            self.content.append(
                &gtk::Label::builder()
                    .label(
                        ngettext("{} réponse", "{} réponses", count)
                            .replace("{}", &count.to_string()),
                    )
                    .xalign(0.0)
                    .margin_top(12)
                    .css_classes(["heading"])
                    .build(),
            );
            for reply in &post.thread {
                self.content.append(&card::reply_card(reply));
            }
        }
        self.stack.set_visible_child_name("thread");
    }
}
