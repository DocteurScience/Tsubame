//! Cartes de post : une carte par post dans le fil, et le post ouvert en grand
//! dans le volet de discussion.

use adw::prelude::*;
use gettextrs::{gettext, ngettext, pgettext};
use gtk::{gio, glib};

use crate::data::{Media, Post, Reply};
use crate::images;
use crate::interactions::{self, Interaction, Interactions, Toggle};
use std::rc::Rc;

const AVATAR_SIZE: i32 = 40;
const AVATAR_SIZE_LARGE: i32 = 48;

/// Remplace les `{}` d'une chaîne traduite, dans l'ordre.
fn fill(template: String, args: &[&str]) -> String {
    args.iter()
        .fold(template, |acc, arg| acc.replacen("{}", arg, 1))
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .build();
    for class in classes {
        label.add_css_class(class);
    }
    label
}

/// Ligne « nom · heure » en haut d'une carte.
/// Ouvre le profil de `handle` au clic.
fn link_to_profile(button: &gtk::Button, name: &str, handle: &str) {
    button.set_action_name(Some("win.open-profile"));
    button.set_action_target_value(Some(&handle.to_variant()));
    let label = gettext("Voir le profil de {}").replace("{}", name);
    button.set_tooltip_text(Some(&format!("@{handle}")));
    button.update_property(&[gtk::accessible::Property::Label(&label)]);
}

/// Ligne « nom · heure » ; le nom ouvre le profil.
fn meta(name: &str, handle: &str, time: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name_label = label(name, &["heading"]);
    name_label.set_wrap(false);
    name_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    if handle.is_empty() {
        row.append(&name_label);
    } else {
        let button = gtk::Button::builder()
            .child(&name_label)
            .css_classes(["flat", "author-link"])
            .build();
        link_to_profile(&button, name, handle);
        row.append(&button);
    }
    let time = label(time, &["dim-label", "caption"]);
    time.set_wrap(false);
    row.append(&time);
    row
}

/// Largeur approximative d'une image dans une carte, pour en déduire la
/// hauteur à réserver d'après ses proportions.
const MEDIA_WIDTH: i32 = 330;

fn badge(text: &str, accessible: &str, halign: gtk::Align, valign: gtk::Align) -> gtk::Label {
    let badge = gtk::Label::builder()
        .label(text)
        .halign(halign)
        .valign(valign)
        .margin_start(8)
        .margin_end(8)
        .margin_top(8)
        .margin_bottom(8)
        .css_classes(["alt-badge"])
        .build();
    badge.update_property(&[gtk::accessible::Property::Label(accessible)]);
    badge
}

/// Image jointe : la miniature, chargée en arrière-plan, ou un emplacement
/// vide pour les faux posts. Pastille « ALT » quand un texte alternatif
/// existe, « +N » quand le post a d'autres images.
fn media(media: &Media, warning: Option<String>) -> gtk::Widget {
    let height = media
        .aspect_ratio
        .filter(|(width, height)| *width > 0 && *height > 0)
        .map(|(width, height)| (MEDIA_WIDTH as i64 * height as i64 / width as i64) as i32)
        .unwrap_or(220)
        .clamp(120, 420);
    let description = if media.alt.is_empty() {
        gettext("Image sans texte alternatif")
    } else {
        media.alt.clone()
    };

    let content: gtk::Widget = match &media.thumb {
        Some(url) => {
            let picture = gtk::Picture::builder()
                .content_fit(gtk::ContentFit::Cover)
                .can_shrink(true)
                .height_request(height)
                .alternative_text(&description)
                .css_classes(["post-media"])
                .build();
            let weak = picture.downgrade();
            images::load(url, move |texture| {
                if let Some(picture) = weak.upgrade() {
                    picture.set_paintable(Some(texture));
                }
            });
            if media.images.is_empty() {
                picture.upcast()
            } else {
                // Un clic ouvre la visionneuse, sans ouvrir la discussion.
                let button = gtk::Button::builder()
                    .child(&picture)
                    .tooltip_text(gettext("Agrandir l'image"))
                    .css_classes(["flat", "media-button"])
                    .build();
                button.update_property(&[gtk::accessible::Property::Label(&fill(
                    gettext("Agrandir l'image : {}"),
                    &[&description],
                ))]);
                let images = media.images.clone();
                button.connect_clicked(move |button| {
                    crate::viewer::present(button, images.clone(), 0);
                });
                button.upcast()
            }
        }
        None => {
            let placeholder = gtk::Image::builder()
                .icon_name("image-x-generic-symbolic")
                .pixel_size(48)
                .height_request(height)
                .css_classes(["post-media", "dim-label"])
                .build();
            placeholder.update_property(&[gtk::accessible::Property::Label(&description)]);
            placeholder.upcast()
        }
    };
    let overlay = gtk::Overlay::builder()
        .child(&content)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["post-media-frame"])
        .build();
    if !media.alt.is_empty() {
        let alt = badge(
            &gettext("ALT"),
            &fill(gettext("Texte alternatif : {}"), &[&media.alt]),
            gtk::Align::End,
            gtk::Align::End,
        );
        alt.set_tooltip_text(Some(&media.alt));
        overlay.add_overlay(&alt);
    }
    if media.more > 0 {
        overlay.add_overlay(&badge(
            &format!("+{}", media.more),
            &ngettext("{} autre image", "{} autres images", media.more)
                .replace("{}", &media.more.to_string()),
            gtk::Align::End,
            gtk::Align::Start,
        ));
    }
    if let Some(playlist) = &media.video {
        add_play_button(&overlay, playlist, &description, height);
    }
    if let Some(warning) = warning {
        add_warning(&overlay, &content, &warning);
    }
    overlay.upcast()
}

/// Vidéo : la vignette porte « Lire la vidéo » ; un clic la remplace par le
/// lecteur de GTK (GStreamer), qui lit la liste de lecture HLS.
fn add_play_button(overlay: &gtk::Overlay, playlist: &str, description: &str, height: i32) {
    let play = gtk::Button::builder()
        .icon_name("media-playback-start-symbolic")
        .tooltip_text(gettext("Lire la vidéo"))
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["circular", "osd", "video-play"])
        .build();
    play.update_property(&[gtk::accessible::Property::Label(&gettext("Lire la vidéo"))]);
    overlay.add_overlay(&play);
    let (playlist, description) = (playlist.to_owned(), description.to_owned());
    let weak = overlay.downgrade();
    play.connect_clicked(move |play| {
        let Some(overlay) = weak.upgrade() else {
            return;
        };
        let video = gtk::Video::builder()
            .autoplay(true)
            .height_request(height)
            .build();
        video.update_property(&[gtk::accessible::Property::Label(&description)]);
        video.set_file(Some(&gio::File::for_uri(&playlist)));
        // Lecture impossible (format, réseau) : le dire à la place.
        if let Some(stream) = video.media_stream() {
            let weak = overlay.downgrade();
            stream.connect_error_notify(move |stream| {
                let (Some(overlay), Some(error)) = (weak.upgrade(), stream.error()) else {
                    return;
                };
                glib::g_debug!("tsubame", "vidéo : {error}");
                let message = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .spacing(8)
                    .height_request(height)
                    .css_classes(["post-media", "dim-label"])
                    .build();
                message.append(
                    &gtk::Image::builder()
                        .icon_name("video-x-generic-symbolic")
                        .pixel_size(32)
                        .vexpand(true)
                        .valign(gtk::Align::End)
                        .build(),
                );
                message.append(
                    &gtk::Label::builder()
                        .label(gettext("Cette vidéo ne peut pas être lue."))
                        .wrap(true)
                        .vexpand(true)
                        .valign(gtk::Align::Start)
                        .build(),
                );
                overlay.set_child(Some(&message));
            });
        }
        overlay.set_child(Some(&video));
        play.set_visible(false);
        video.grab_focus();
    });
}

/// Image floutée sous un avertissement, avec « Afficher » pour la voir.
fn add_warning(overlay: &gtk::Overlay, content: &gtk::Widget, warning: &str) {
    content.add_css_class("media-blurred");
    // Le texte alternatif décrit ce qui est caché : on ne le lit pas avant.
    content.set_can_target(false);
    let badges: Vec<gtk::Widget> =
        std::iter::successors(content.next_sibling(), |w| w.next_sibling()).collect();
    for badge in &badges {
        badge.set_visible(false);
    }
    let shield = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Fill)
        .css_classes(["media-warning"])
        .build();
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .vexpand(true)
        .valign(gtk::Align::Center)
        .build();
    inner.append(
        &gtk::Image::builder()
            .icon_name("dialog-warning-symbolic")
            .pixel_size(24)
            .build(),
    );
    let title = gtk::Label::builder()
        .label(warning)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["heading"])
        .build();
    inner.append(&title);
    let show = gtk::Button::builder()
        .label(pgettext("voir une image floutée", "Afficher"))
        .halign(gtk::Align::Center)
        .css_classes(["pill"])
        .build();
    show.update_property(&[gtk::accessible::Property::Description(warning)]);
    inner.append(&show);
    shield.append(&inner);
    overlay.add_overlay(&shield);
    let content = content.clone();
    show.connect_clicked(move |_| {
        content.remove_css_class("media-blurred");
        content.set_can_target(true);
        for badge in &badges {
            badge.set_visible(true);
        }
        shield.set_visible(false);
    });
}

/// Bouton d'action à icône et compteur. Le nom accessible décrit l'action et
/// le nombre, l'icône seule ne suffit pas.
fn stat_button(icon: &str, count: u32, accessible: String) -> gtk::Button {
    let content = adw::ButtonContent::builder()
        .icon_name(icon)
        .label(count.to_string())
        .build();
    let button = gtk::Button::builder()
        .child(&content)
        .tooltip_text(&accessible)
        .css_classes(["flat", "post-stat"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(&accessible)]);
    button
}

fn stats(post: &Post) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.add_css_class("post-stats");
    let n = |count: u32| count.to_string();
    let entry = interactions::register(post);
    let reply = stat_button(
        "mail-reply-sender-symbolic",
        post.replies,
        fill(
            ngettext(
                "Répondre, {} réponse",
                "Répondre, {} réponses",
                post.replies,
            ),
            &[&n(post.replies)],
        ),
    );
    reply.set_action_name(Some("win.reply"));
    reply.set_action_target_value(Some(&entry.key.to_variant()));
    row.append(&reply);
    row.append(&interaction_button(&entry, Interaction::Repost, false));
    row.append(&interaction_button(&entry, Interaction::Like, false));
    row.append(&interaction_button(&entry, Interaction::Bookmark, false));
    row.append(&more_menu(post, &entry.key));
    row
}

/// Menu « Plus d'actions » d'une carte : copier le lien, masquer ou
/// bloquer l'auteur.
fn more_menu(post: &Post, key: &str) -> gtk::MenuButton {
    let item = |label: String, action: &str, target: &str| {
        let item = gio::MenuItem::new(Some(&label), None);
        item.set_action_and_target_value(Some(action), Some(&target.to_variant()));
        item
    };
    let menu = gio::Menu::new();
    let sharing = gio::Menu::new();
    sharing.append_item(&item(gettext("Copier le lien"), "win.copy-link", key));
    menu.append_section(None, &sharing);
    if crate::interactions::is_mine(post) {
        let mine = gio::Menu::new();
        mine.append_item(&item(
            if post.pinned {
                gettext("Retirer du haut du profil")
            } else {
                gettext("Épingler sur le profil")
            },
            "win.pin-post",
            key,
        ));
        mine.append_item(&item(gettext("Supprimer le post…"), "win.delete-post", key));
        menu.append_section(None, &mine);
    }
    let reporting = gio::Menu::new();
    {
        reporting.append_item(&item(gettext("Signaler le post…"), "win.report-post", key));
    }
    if !post.handle.is_empty() && !crate::interactions::is_mine(post) {
        let author = gio::Menu::new();
        author.append_item(&item(
            gettext("Masquer @{}").replace("{}", &post.handle),
            "win.mute-author",
            &post.handle,
        ));
        author.append_item(&item(
            gettext("Bloquer @{}").replace("{}", &post.handle),
            "win.block-author",
            &post.handle,
        ));
        menu.append_section(None, &author);
        reporting.append_item(&item(
            gettext("Signaler @{}…").replace("{}", &post.handle),
            "win.report-author",
            &post.handle,
        ));
    }
    menu.append_section(None, &reporting);
    let button = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .menu_model(&menu)
        .tooltip_text(gettext("Plus d'actions"))
        .css_classes(["flat", "post-stat"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(&gettext("Plus d'actions"))]);
    button
}

/// Libellé accessible (et info-bulle) d'un bouton J'aime ou Reposter.
fn interaction_label(which: Interaction, toggle: &Toggle) -> String {
    let count = toggle.count;
    let text = match (which, toggle.active) {
        (Interaction::Like, false) => ngettext("Aimer, {} j'aime", "Aimer, {} j'aime", count),
        (Interaction::Like, true) => ngettext(
            "Ne plus aimer, {} j'aime",
            "Ne plus aimer, {} j'aime",
            count,
        ),
        (Interaction::Repost, false) => ngettext(
            "Reposter ou citer, {} repost",
            "Reposter ou citer, {} reposts",
            count,
        ),
        (Interaction::Repost, true) => ngettext(
            "Annuler le repost ou citer, {} repost",
            "Annuler le repost ou citer, {} reposts",
            count,
        ),
        (Interaction::Bookmark, false) => gettext("Ajouter aux signets"),
        (Interaction::Bookmark, true) => gettext("Retirer des signets"),
    };
    fill(text, &[&count.to_string()])
}

/// Texte du bouton dans la discussion : « J'aime · 48 », « Aimé · 49 »…
fn interaction_text(which: Interaction, toggle: &Toggle) -> String {
    let text = match (which, toggle.active) {
        (Interaction::Like, false) => gettext("J'aime · {}"),
        (Interaction::Like, true) => gettext("Aimé · {}"),
        (Interaction::Repost, false) => gettext("Reposter · {}"),
        (Interaction::Repost, true) => gettext("Reposté · {}"),
        (Interaction::Bookmark, false) => gettext("Signet"),
        (Interaction::Bookmark, true) => gettext("Dans les signets"),
    };
    fill(text, &[&toggle.count.to_string()])
}

/// Bouton J'aime ou Reposter, relié à l'état partagé du post : il suit les
/// changements faits depuis une autre carte du même post.
/// Menu du bouton Reposter : reposter (ou annuler), citer.
fn repost_menu(key: &str, active: bool) -> gio::Menu {
    let menu = gio::Menu::new();
    let item = |label: String, action: &str| {
        let item = gio::MenuItem::new(Some(&label), None);
        item.set_action_and_target_value(Some(action), Some(&key.to_variant()));
        item
    };
    menu.append_item(&item(
        if active {
            gettext("Annuler le repost")
        } else {
            gettext("Reposter")
        },
        "win.toggle-repost",
    ));
    menu.append_item(&item(gettext("Citer…"), "win.quote"));
    menu
}

/// Bouton J'aime, Reposter ou Signet. Reposter ouvre un menu (reposter,
/// citer), comme sur Bluesky.
fn interaction_button(entry: &Rc<Interactions>, which: Interaction, detailed: bool) -> gtk::Widget {
    let (icon, action) = match which {
        Interaction::Like => ("emblem-favorite-symbolic", "win.toggle-like"),
        Interaction::Repost => ("media-playlist-repeat-symbolic", "win.toggle-repost"),
        Interaction::Bookmark => ("user-bookmarks-symbolic", "win.toggle-bookmark"),
    };
    let content = adw::ButtonContent::builder().icon_name(icon).build();
    let button: gtk::Widget = if which == Interaction::Repost {
        gtk::MenuButton::builder()
            .child(&content)
            .menu_model(&repost_menu(&entry.key, entry.get(which).active))
            .build()
            .upcast()
    } else {
        gtk::Button::builder()
            .child(&content)
            .action_name(action)
            .action_target(&entry.key.to_variant())
            .build()
            .upcast()
    };
    if detailed {
        button.set_halign(gtk::Align::Start);
        button.add_css_class("post-action");
    } else {
        button.add_css_class("flat");
        button.add_css_class("post-stat");
    }
    let key = entry.key.clone();
    let update = move |button: &gtk::Widget, content: &adw::ButtonContent, toggle: &Toggle| {
        if let Some(menu_button) = button.downcast_ref::<gtk::MenuButton>() {
            menu_button.set_menu_model(Some(&repost_menu(&key, toggle.active)));
        }
        content.set_label(&if detailed {
            interaction_text(which, toggle)
        } else if which == Interaction::Bookmark {
            // Le signet est privé : pas de compteur.
            String::new()
        } else {
            toggle.count.to_string()
        });
        let label = interaction_label(which, toggle);
        button.set_tooltip_text(Some(&label));
        button.update_property(&[gtk::accessible::Property::Label(&label)]);
        button.update_state(&[gtk::accessible::State::Pressed(if toggle.active {
            gtk::AccessibleTristate::True
        } else {
            gtk::AccessibleTristate::False
        })]);
        if toggle.active {
            button.add_css_class("active");
        } else {
            button.remove_css_class("active");
        }
    };
    update(&button, &content, &entry.get(which));
    let weak_button = button.downgrade();
    let weak_content = content.downgrade();
    entry.connect(move |changed, toggle| {
        if changed == which
            && let (Some(button), Some(content)) = (weak_button.upgrade(), weak_content.upgrade())
        {
            update(&button, &content, toggle);
        }
    });
    // La carte garde l'état du post en vie tant qu'elle est affichée.
    let entry = entry.clone();
    button.connect_destroy(move |_| {
        let _ = &entry;
    });
    button
}

fn card_frame(
    avatar_size: i32,
    name: &str,
    handle: &str,
    avatar_url: Option<&str>,
) -> (gtk::Box, gtk::Box) {
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .css_classes(["card", "post-card"])
        .build();
    let avatar = adw::Avatar::builder()
        .size(avatar_size)
        .text(name)
        .show_initials(true)
        .valign(gtk::Align::Start)
        .build();
    // Les initiales restent affichées tant que l'image n'est pas là.
    if let Some(url) = avatar_url {
        let weak = avatar.downgrade();
        images::load(url, move |texture| {
            if let Some(avatar) = weak.upgrade() {
                avatar.set_custom_image(Some(texture));
            }
        });
    }
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .build();
    // L'avatar ouvre aussi le profil.
    if handle.is_empty() {
        card.append(&avatar);
    } else {
        let button = gtk::Button::builder()
            .child(&avatar)
            .valign(gtk::Align::Start)
            .css_classes(["flat", "circular", "avatar-button"])
            .build();
        link_to_profile(&button, name, handle);
        card.append(&button);
    }
    card.append(&body);
    (card, body)
}

/// Carte d'un post dans le fil.
pub fn post_card(post: &Post) -> gtk::Box {
    let (card, body) = card_frame(
        AVATAR_SIZE,
        &post.name,
        &post.handle,
        post.avatar.as_deref(),
    );
    if post.unread {
        card.add_css_class("unread");
    }
    if let Some(context) = &post.context {
        body.append(&label(context, &["dim-label", "caption"]));
    }
    body.append(&meta(&post.name, &post.handle, &post.time));
    if !post.text.is_empty() {
        body.append(&label(&post.text, &[]));
    }
    if let Some(m) = &post.media {
        body.append(&media(m, crate::moderation::media_warning(post)));
    }
    if let Some(quoted) = &post.quote {
        body.append(&quote_card(quoted));
    }
    body.append(&stats(post));
    card
}

/// Post cité, en petite carte dans celle du post. Un clic l'ouvre ; un
/// post supprimé ou invisible n'affiche que la raison.
fn quote_card(quoted: &Post) -> gtk::Widget {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();
    if quoted.handle.is_empty() {
        body.append(&label(&quoted.text, &["dim-label"]));
        let frame = gtk::Box::builder().css_classes(["quote-card"]).build();
        frame.append(&body);
        return frame.upcast();
    }
    let header = gtk::Box::builder().spacing(6).build();
    let avatar = adw::Avatar::builder()
        .size(20)
        .text(&quoted.name)
        .show_initials(true)
        .build();
    if let Some(url) = &quoted.avatar {
        let weak = avatar.downgrade();
        images::load(url, move |texture| {
            if let Some(avatar) = weak.upgrade() {
                avatar.set_custom_image(Some(texture));
            }
        });
    }
    header.append(&avatar);
    let name = label(&quoted.name, &["heading"]);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_wrap(false);
    header.append(&name);
    let detail = label(
        &format!("@{} · {}", quoted.handle, quoted.time),
        &["dim-label", "caption"],
    );
    detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
    detail.set_wrap(false);
    detail.set_hexpand(true);
    header.append(&detail);
    body.append(&header);
    if !quoted.text.is_empty() {
        let text = label(&quoted.text, &[]);
        text.set_lines(6);
        text.set_ellipsize(gtk::pango::EllipsizeMode::End);
        body.append(&text);
    }
    // Le post cité peut déjà être affiché ailleurs : on garde son état
    // (j'aime, repost) plutôt que de le remplacer par la vue de citation,
    // qui n'en a pas.
    let key = crate::interactions::key(quoted);
    let entry =
        crate::interactions::find(&key).unwrap_or_else(|| crate::interactions::register(quoted));
    let button = gtk::Button::builder()
        .child(&body)
        .css_classes(["quote-card"])
        .action_name("win.open-post")
        .action_target(&entry.key.to_variant())
        .build();
    button.update_property(&[gtk::accessible::Property::Label(&fill(
        gettext("Post cité de {} : {}"),
        &[&quoted.name, &quoted.text],
    ))]);
    // La carte garde l'entrée en vie, pour que le clic la retrouve.
    button.connect_destroy(move |_| {
        let _ = &entry;
    });
    button.upcast()
}

/// Carte d'une réponse dans le volet de discussion.
pub fn reply_card(reply: &Reply) -> gtk::Box {
    let (card, body) = card_frame(
        AVATAR_SIZE,
        &reply.name,
        &reply.handle,
        reply.avatar.as_deref(),
    );
    card.set_tooltip_text(Some(&format!("@{}", reply.handle)));
    body.append(&meta(&reply.name, &reply.handle, &reply.time));
    body.append(&label(&reply.text, &[]));
    card
}

/// Bouton d'action du post ouvert : icône et texte.
fn action_button(icon: &str, text: String) -> gtk::Button {
    gtk::Button::builder()
        .halign(gtk::Align::Start)
        .child(
            &adw::ButtonContent::builder()
                .icon_name(icon)
                .label(text)
                .build(),
        )
        .build()
}

/// « 31 j'aime · 2 reposts » sous le post ouvert : chaque compteur ouvre la
/// liste des comptes concernés. Les compteurs suivent les interactions.
fn people_links(entry: &Rc<Interactions>) -> gtk::Box {
    let row = gtk::Box::builder().spacing(4).margin_top(6).build();
    let link = |action: &str| {
        let label = gtk::Label::builder().use_markup(true).build();
        let button = gtk::Button::builder()
            .child(&label)
            .action_name(action)
            .action_target(&entry.key.to_variant())
            .css_classes(["flat", "profile-count"])
            .build();
        (button, label)
    };
    let (likes, likes_label) = link("win.show-likers");
    let (reposts, reposts_label) = link("win.show-reposters");
    row.append(&likes);
    row.append(&reposts);
    // Références faibles : l'état du post survit aux cartes.
    let widgets = [
        (likes.downgrade(), likes_label.downgrade()),
        (reposts.downgrade(), reposts_label.downgrade()),
    ];
    let update = move |which: Interaction, toggle: &Toggle| {
        let index = match which {
            Interaction::Like => 0,
            Interaction::Repost => 1,
            Interaction::Bookmark => return,
        };
        let (Some(button), Some(label)) = (widgets[index].0.upgrade(), widgets[index].1.upgrade())
        else {
            return;
        };
        // Chaînes littérales, pour xgettext ; le nombre en gras.
        let (template, tooltip) = match which {
            Interaction::Like => (
                ngettext("{} j'aime", "{} j'aime", toggle.count),
                gettext("Voir qui a aimé"),
            ),
            _ => (
                ngettext("{} repost", "{} reposts", toggle.count),
                gettext("Voir qui a reposté"),
            ),
        };
        label.set_label(
            &gtk::glib::markup_escape_text(&template)
                .replace("{}", &format!("<b>{}</b>", toggle.count)),
        );
        button.set_tooltip_text(Some(&tooltip));
        button.set_visible(toggle.count > 0);
    };
    update(Interaction::Like, &entry.get(Interaction::Like));
    update(Interaction::Repost, &entry.get(Interaction::Repost));
    entry.connect(move |which, toggle| update(which, toggle));
    row
}

/// Post ouvert en grand en tête du volet de discussion.
pub fn post_detail(post: &Post) -> gtk::Box {
    let (card, body) = card_frame(
        AVATAR_SIZE_LARGE,
        &post.name,
        &post.handle,
        post.avatar.as_deref(),
    );
    card.add_css_class("post-detail");

    body.append(&label(&post.name, &["title-4"]));
    body.append(&label(
        &format!("@{} · {}", post.handle, post.date),
        &["dim-label", "caption"],
    ));
    body.append(&label(&post.text, &["post-detail-text"]));
    if let Some(m) = &post.media {
        body.append(&media(m, crate::moderation::media_warning(post)));
    }
    if let Some(quoted) = &post.quote {
        body.append(&quote_card(quoted));
    }

    // Les boutons passent à la ligne quand le volet est étroit.
    let actions = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(6)
        .row_spacing(6)
        .max_children_per_line(4)
        .margin_top(8)
        .build();
    let entry = interactions::register(post);
    body.append(&people_links(&entry));
    let reply = action_button("mail-reply-sender-symbolic", gettext("Répondre"));
    reply.set_action_name(Some("win.reply"));
    reply.set_action_target_value(Some(&entry.key.to_variant()));
    actions.append(&reply);
    actions.append(&interaction_button(&entry, Interaction::Repost, true));
    actions.append(&interaction_button(&entry, Interaction::Like, true));
    actions.append(&interaction_button(&entry, Interaction::Bookmark, true));
    // Seuls les boutons prennent le focus, pas les cases du FlowBox.
    let mut child = actions.first_child();
    while let Some(widget) = child {
        widget.set_focusable(false);
        child = widget.next_sibling();
    }
    body.append(&actions);
    card
}
