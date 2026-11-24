//! Cartes de post : une carte par post dans le fil, et le post ouvert en grand
//! dans le volet de discussion.

use adw::prelude::*;
use gettextrs::{gettext, ngettext, pgettext};
use gtk::{gio, glib};

use crate::data::{Image, Link, Media, Post, Reply};
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

/// Texte d'un post, avec ses liens, mentions et mots-dièse cliquables. Les
/// adresses web s'ouvrent dans le navigateur ; mentions et mots-dièse
/// passent par les actions `win.open-profile` et `win.open-tag`.
fn rich_text(text: &str, facets: &[crate::richtext::Facet], classes: &[&str]) -> gtk::Label {
    use crate::richtext::{PROFILE_LINK, TAG_LINK, markup};

    let label = label(text, classes);
    if facets.is_empty() {
        return label;
    }
    label.set_markup(&markup(text, facets));
    label.connect_activate_link(|label, uri| {
        let (action, value) = if let Some(tag) = uri.strip_prefix(TAG_LINK) {
            ("win.open-tag", tag)
        } else if let Some(actor) = uri.strip_prefix(PROFILE_LINK) {
            ("win.open-profile", actor)
        } else {
            return glib::Propagation::Proceed;
        };
        let _ = label.activate_action(action, Some(&value.to_variant()));
        glib::Propagation::Stop
    });
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

    // Plusieurs images : une grille (2, 3 ou 4 vignettes), chacune avec sa
    // pastille ALT ; les pastilles de l'ensemble ne servent plus.
    let grid = media.images.len() > 1 && media.video.is_none();
    let content: gtk::Widget = match &media.thumb {
        Some(_) if grid => image_grid(&media.images),
        Some(url) => {
            // Dans un bouton « Agrandir l'image : … », l'image n'est pas
            // annoncée une seconde fois ; seule, elle porte son texte
            // alternatif comme nom (`alternative-text` n'en donne pas).
            let in_button = !media.images.is_empty();
            let picture = gtk::Picture::builder()
                .content_fit(gtk::ContentFit::Cover)
                .can_shrink(true)
                .height_request(height)
                .alternative_text(&description)
                .accessible_role(if in_button {
                    gtk::AccessibleRole::Presentation
                } else {
                    gtk::AccessibleRole::Img
                })
                .css_classes(["post-media"])
                .build();
            if !in_button {
                picture.update_property(&[gtk::accessible::Property::Label(&description)]);
            }
            let weak = picture.downgrade();
            images::load(url, move |texture| {
                if let Some(picture) = weak.upgrade() {
                    picture.set_paintable(Some(texture));
                }
            });
            if media.images.is_empty() {
                picture.upcast()
            } else {
                viewer_button(&picture, &media.images, 0, &description).upcast()
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
    if !media.alt.is_empty() && !grid {
        let alt = badge(
            &gettext("ALT"),
            &fill(gettext("Texte alternatif : {}"), &[&media.alt]),
            gtk::Align::End,
            gtk::Align::End,
        );
        alt.set_tooltip_text(Some(&media.alt));
        overlay.add_overlay(&alt);
    }
    if media.more > 0 && !grid {
        overlay.add_overlay(&badge(
            &format!("+{}", media.more),
            &ngettext("{} autre image", "{} autres images", media.more)
                .replace("{}", &media.more.to_string()),
            gtk::Align::End,
            gtk::Align::Start,
        ));
    }
    match &media.video {
        Some(playlist) if media.gif => add_gif_button(&overlay, playlist, &description, height),
        Some(playlist) => add_play_button(&overlay, playlist, &description, height),
        None => {}
    }
    if let Some(warning) = warning {
        add_warning(&overlay, &content, &warning);
    }
    overlay.upcast()
}

/// Bouton autour d'une image : un clic ouvre la visionneuse sur l'image
/// `index`, sans ouvrir la discussion.
fn viewer_button(
    child: &impl IsA<gtk::Widget>,
    images: &[Image],
    index: usize,
    description: &str,
) -> gtk::Button {
    let button = gtk::Button::builder()
        .child(child)
        .tooltip_text(gettext("Agrandir l'image"))
        .css_classes(["flat", "media-button"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(&fill(
        gettext("Agrandir l'image : {}"),
        &[description],
    ))]);
    let images = images.to_vec();
    button.connect_clicked(move |button| {
        crate::viewer::present(button, images.clone(), index);
    });
    button
}

/// Place des vignettes dans la grille : (colonne, ligne, lignes occupées).
/// Deux images côte à côte ; trois : une grande à gauche, deux à droite ;
/// quatre : deux sur deux.
pub fn grid_cells(count: usize) -> Vec<(i32, i32, i32)> {
    match count {
        0 | 1 => vec![(0, 0, 2)],
        2 => vec![(0, 0, 2), (1, 0, 2)],
        3 => vec![(0, 0, 2), (1, 0, 1), (1, 1, 1)],
        _ => vec![(0, 0, 1), (1, 0, 1), (0, 1, 1), (1, 1, 1)],
    }
}

/// Hauteur de la grille d'images, en pixels.
const GRID_HEIGHT: i32 = 240;
const GRID_SPACING: i32 = 4;

fn image_grid(images: &[Image]) -> gtk::Widget {
    let grid = gtk::Grid::builder()
        .column_spacing(GRID_SPACING)
        .row_spacing(GRID_SPACING)
        .column_homogeneous(true)
        .build();
    let count = images.len().min(4);
    for (index, (column, row, rows)) in grid_cells(count).into_iter().enumerate() {
        let image = &images[index];
        let height = if rows == 2 {
            GRID_HEIGHT
        } else {
            (GRID_HEIGHT - GRID_SPACING) / 2
        };
        let description = if image.alt.is_empty() {
            gettext("Image sans texte alternatif")
        } else {
            image.alt.clone()
        };
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .height_request(height)
            .alternative_text(&description)
            // Le bouton autour porte déjà le texte alternatif.
            .accessible_role(gtk::AccessibleRole::Presentation)
            .css_classes(["post-media"])
            .build();
        let weak = picture.downgrade();
        images::load(&image.thumb, move |texture| {
            if let Some(picture) = weak.upgrade() {
                picture.set_paintable(Some(texture));
            }
        });
        let tile = gtk::Overlay::builder().child(&picture).build();
        if !image.alt.is_empty() {
            let alt = badge(
                &gettext("ALT"),
                &fill(gettext("Texte alternatif : {}"), &[&image.alt]),
                gtk::Align::End,
                gtk::Align::End,
            );
            alt.set_tooltip_text(Some(&image.alt));
            tile.add_overlay(&alt);
        }
        let button = viewer_button(&tile, images, index, &description);
        grid.attach(&button, column, row, 1, rows);
    }
    grid.upcast()
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
                        .accessible_role(gtk::AccessibleRole::Presentation)
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

/// GIF (vidéo `presentation: gif`) : « Lire le GIF » ; un clic le lit en
/// boucle, sans son ni commandes ; un autre clic le met en pause. Pas de
/// lecture automatique : rien ne bouge sans le demander.
fn add_gif_button(overlay: &gtk::Overlay, playlist: &str, description: &str, height: i32) {
    overlay.add_overlay(&badge(
        &gettext("GIF"),
        &gettext("GIF animé"),
        gtk::Align::Start,
        gtk::Align::End,
    ));
    let play = gtk::Button::builder()
        .icon_name("media-playback-start-symbolic")
        .tooltip_text(gettext("Lire le GIF"))
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["circular", "osd", "video-play"])
        .build();
    play.update_property(&[gtk::accessible::Property::Label(&gettext("Lire le GIF"))]);
    overlay.add_overlay(&play);
    let (playlist, description) = (playlist.to_owned(), description.to_owned());
    let weak = overlay.downgrade();
    play.connect_clicked(move |play| {
        let Some(overlay) = weak.upgrade() else {
            return;
        };
        let media = gtk::MediaFile::for_file(&gio::File::for_uri(&playlist));
        media.set_loop(true);
        media.set_muted(true);
        let picture = gtk::Picture::builder()
            .paintable(&media)
            .height_request(height)
            .content_fit(gtk::ContentFit::Cover)
            .build();
        picture.update_property(&[gtk::accessible::Property::Label(&description)]);
        // Pause et reprise au clic, ou avec Espace sur le bouton qui l'entoure.
        let toggle = gtk::Button::builder()
            .child(&picture)
            .tooltip_text(gettext("Mettre en pause ou reprendre le GIF"))
            .css_classes(["flat", "gif-toggle"])
            .build();
        toggle.update_property(&[gtk::accessible::Property::Label(
            &gettext("GIF : {}").replace("{}", &description),
        )]);
        let playing = media.clone();
        toggle.connect_clicked(move |_| {
            if playing.is_playing() {
                playing.pause();
            } else {
                playing.play();
            }
        });
        media.play();
        overlay.set_child(Some(&toggle));
        play.set_visible(false);
        toggle.grab_focus();
    });
}

/// Texte d'un post sans image, caché sous un avertissement : une ligne
/// « ⚠ étiquette » et « Afficher », qui révèle le texte.
fn warned_text(text: gtk::Label, warning: &str) -> gtk::Widget {
    let bar = gtk::Box::builder()
        .spacing(8)
        .css_classes(["text-warning"])
        .build();
    bar.append(&decorative_icon("dialog-warning-symbolic"));
    bar.append(
        &gtk::Label::builder()
            .label(warning)
            .wrap(true)
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["heading"])
            .build(),
    );
    let show = gtk::Button::builder()
        .label(pgettext("voir une image floutée", "Afficher"))
        .valign(gtk::Align::Center)
        .css_classes(["pill"])
        .build();
    show.update_property(&[gtk::accessible::Property::Description(warning)]);
    bar.append(&show);
    let revealer = gtk::Revealer::builder().child(&text).build();
    let holder = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    holder.append(&bar);
    holder.append(&revealer);
    show.connect_clicked(move |show| {
        revealer.set_reveal_child(true);
        show.set_visible(false);
    });
    holder.upcast()
}

/// Texte du post, caché sous l'avertissement d'une étiquette s'il n'a pas
/// d'image à flouter.
fn post_text(post: &Post, classes: &[&str]) -> gtk::Widget {
    let text = rich_text(&post.text, &post.facets, classes);
    match crate::moderation::media_warning(post) {
        Some(warning) if post.media.is_none() => warned_text(text, &warning),
        _ => text.upcast(),
    }
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
            .accessible_role(gtk::AccessibleRole::Presentation)
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

/// Icône décorative : les lecteurs d'écran l'ignorent, le nom est ailleurs.
pub fn decorative_icon(icon: &str) -> gtk::Image {
    gtk::Image::builder()
        .icon_name(icon)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build()
}

/// Icône et texte d'un bouton. Pas d'`AdwButtonContent` ici : il relie le
/// bouton à son étiquette visible (`labelled-by`), qui passe avant le nom
/// accessible : Orca lirait « 31 » au lieu de « Aimer, 31 j'aime ».
fn icon_label(icon: &str, text: &str) -> (gtk::Box, gtk::Label) {
    let content = gtk::Box::builder()
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    content.append(&decorative_icon(icon));
    let label = gtk::Label::builder()
        .label(text)
        .visible(!text.is_empty())
        .build();
    content.append(&label);
    (content, label)
}

/// Bouton d'action à icône et compteur. Le nom accessible décrit l'action et
/// le nombre, l'icône seule ne suffit pas.
fn stat_button(icon: &str, count: u32, accessible: String) -> gtk::Button {
    let (content, _) = icon_label(icon, &count.to_string());
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
    if post.uri.is_some() {
        sharing.append_item(&item(gettext("Envoyer en message…"), "win.send-post", key));
    }
    if !post.text.trim().is_empty() {
        sharing.append_item(&item(gettext("Traduire"), "win.translate", key));
    }
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
    let (content, text) = icon_label(icon, "");
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
    let update = move |button: &gtk::Widget, text: &gtk::Label, toggle: &Toggle| {
        if let Some(menu_button) = button.downcast_ref::<gtk::MenuButton>() {
            menu_button.set_menu_model(Some(&repost_menu(&key, toggle.active)));
        }
        let visible = if detailed {
            interaction_text(which, toggle)
        } else if which == Interaction::Bookmark {
            // Le signet est privé : pas de compteur.
            String::new()
        } else {
            toggle.count.to_string()
        };
        text.set_label(&visible);
        text.set_visible(!visible.is_empty());
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
    update(&button, &text, &entry.get(which));
    let weak_button = button.downgrade();
    let weak_text = text.downgrade();
    entry.connect(move |changed, toggle| {
        if changed == which
            && let (Some(button), Some(text)) = (weak_button.upgrade(), weak_text.upgrade())
        {
            update(&button, &text, toggle);
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
    // L'avatar ouvre aussi le profil, à la souris : au clavier, le nom
    // suffit (un seul arrêt de tabulation par profil).
    if handle.is_empty() {
        card.append(&avatar);
    } else {
        let button = gtk::Button::builder()
            .child(&avatar)
            .valign(gtk::Align::Start)
            .focusable(false)
            .accessible_role(gtk::AccessibleRole::Presentation)
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
        body.append(&post_text(post, &[]));
    }
    if let Some(m) = &post.media {
        body.append(&media(m, crate::moderation::media_warning(post)));
    }
    if let Some(link) = &post.link {
        body.append(&link_card(link));
    }
    if let Some(quoted) = &post.quote {
        body.append(&quote_card(quoted));
    }
    body.append(&stats(post));
    card
}

/// Carte d'un lien joint : image d'aperçu, domaine, titre, description.
/// Un clic ouvre le lien dans le navigateur.
fn link_card(link: &Link) -> gtk::Widget {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    if let Some(url) = &link.thumb {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .height_request(150)
            // Illustration : le titre du lien suffit.
            .accessible_role(gtk::AccessibleRole::Presentation)
            .css_classes(["post-media"])
            .build();
        let weak = picture.downgrade();
        images::load(url, move |texture| {
            if let Some(picture) = weak.upgrade() {
                picture.set_paintable(Some(texture));
            }
        });
        body.append(&picture);
    }
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_top(8)
        .margin_bottom(10)
        .margin_start(10)
        .margin_end(10)
        .build();
    let domain = link.domain();
    let domain_label = label(&domain, &["dim-label", "caption"]);
    domain_label.set_wrap(false);
    domain_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&domain_label);
    if !link.title.trim().is_empty() {
        let title = label(link.title.trim(), &["heading"]);
        title.set_lines(2);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&title);
    }
    if !link.description.trim().is_empty() {
        let description = label(link.description.trim(), &["dim-label"]);
        description.set_lines(2);
        description.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&description);
    }
    body.append(&text);
    let button = gtk::Button::builder()
        .child(&body)
        .tooltip_text(&link.uri)
        .css_classes(["link-card"])
        .build();
    let name = if link.title.trim().is_empty() {
        domain.clone()
    } else {
        format!("{} ({domain})", link.title.trim())
    };
    button.update_property(&[gtk::accessible::Property::Label(&fill(
        gettext("Ouvrir le lien : {}"),
        &[&name],
    ))]);
    let uri = link.uri.clone();
    button.connect_clicked(move |button| {
        let window = button.root().and_downcast::<gtk::Window>();
        gtk::UriLauncher::new(&uri).launch(window.as_ref(), gio::Cancellable::NONE, |result| {
            if let Err(error) = result {
                glib::g_debug!("tsubame", "lien : {}", error.message());
            }
        });
    });
    button.upcast()
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
    body.append(&rich_text(&reply.text, &reply.facets, &[]));
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
    body.append(&post_text(post, &["post-detail-text"]));
    if let Some(m) = &post.media {
        body.append(&media(m, crate::moderation::media_warning(post)));
    }
    if let Some(link) = &post.link {
        body.append(&link_card(link));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_grids_fill_two_rows() {
        for count in 1..=4 {
            let cells = grid_cells(count);
            assert_eq!(cells.len(), count.max(1));
            // Chaque colonne occupe exactement deux lignes.
            for column in 0..2 {
                let rows: i32 = cells
                    .iter()
                    .filter(|(c, _, _)| *c == column)
                    .map(|(_, _, r)| r)
                    .sum();
                assert!(rows == 2 || (count == 1 && column == 1 && rows == 0));
            }
        }
    }
}
