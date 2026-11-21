//! Cartes de post : une carte par post dans le fil, et le post ouvert en grand
//! dans le volet de discussion.

use adw::prelude::*;
use gettextrs::{gettext, ngettext};

use crate::data::{Media, Post, Reply};
use crate::images;

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
fn meta(name: &str, time: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name = label(name, &["heading"]);
    name.set_wrap(false);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let time = label(time, &["dim-label", "caption"]);
    time.set_wrap(false);
    row.append(&name);
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
fn media(media: &Media) -> gtk::Widget {
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
            picture.upcast()
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
    overlay.upcast()
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
    row.append(&stat_button(
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
    ));
    row.append(&stat_button(
        "media-playlist-repeat-symbolic",
        post.reposts,
        fill(
            ngettext("Reposter, {} repost", "Reposter, {} reposts", post.reposts),
            &[&n(post.reposts)],
        ),
    ));
    row.append(&stat_button(
        "emblem-favorite-symbolic",
        post.likes,
        fill(
            ngettext("Aimer, {} j'aime", "Aimer, {} j'aime", post.likes),
            &[&n(post.likes)],
        ),
    ));
    row
}

fn card_frame(avatar_size: i32, name: &str, avatar_url: Option<&str>) -> (gtk::Box, gtk::Box) {
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
    card.append(&avatar);
    card.append(&body);
    (card, body)
}

/// Carte d'un post dans le fil.
pub fn post_card(post: &Post) -> gtk::Box {
    let (card, body) = card_frame(AVATAR_SIZE, &post.name, post.avatar.as_deref());
    if post.unread {
        card.add_css_class("unread");
    }
    if let Some(context) = &post.context {
        body.append(&label(context, &["dim-label", "caption"]));
    }
    body.append(&meta(&post.name, &post.time));
    if !post.text.is_empty() {
        body.append(&label(&post.text, &[]));
    }
    if let Some(m) = &post.media {
        body.append(&media(m));
    }
    body.append(&stats(post));
    card
}

/// Carte d'une réponse dans le volet de discussion.
pub fn reply_card(reply: &Reply) -> gtk::Box {
    let (card, body) = card_frame(AVATAR_SIZE, &reply.name, reply.avatar.as_deref());
    card.set_tooltip_text(Some(&format!("@{}", reply.handle)));
    body.append(&meta(&reply.name, &reply.time));
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

/// Post ouvert en grand en tête du volet de discussion.
pub fn post_detail(post: &Post) -> gtk::Box {
    let (card, body) = card_frame(AVATAR_SIZE_LARGE, &post.name, post.avatar.as_deref());
    card.add_css_class("post-detail");

    body.append(&label(&post.name, &["title-4"]));
    body.append(&label(
        &format!("@{} · {}", post.handle, post.date),
        &["dim-label", "caption"],
    ));
    body.append(&label(&post.text, &["post-detail-text"]));
    if let Some(m) = &post.media {
        body.append(&media(m));
    }

    // Les boutons passent à la ligne quand le volet est étroit.
    let actions = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(6)
        .row_spacing(6)
        .max_children_per_line(3)
        .margin_top(8)
        .build();
    actions.append(&action_button(
        "mail-reply-sender-symbolic",
        gettext("Répondre"),
    ));
    actions.append(&action_button(
        "media-playlist-repeat-symbolic",
        fill(gettext("Reposter · {}"), &[&post.reposts.to_string()]),
    ));
    actions.append(&action_button(
        "emblem-favorite-symbolic",
        fill(gettext("J'aime · {}"), &[&post.likes.to_string()]),
    ));
    // Seuls les boutons prennent le focus, pas les cases du FlowBox.
    let mut child = actions.first_child();
    while let Some(widget) = child {
        widget.set_focusable(false);
        child = widget.next_sibling();
    }
    body.append(&actions);
    card
}
