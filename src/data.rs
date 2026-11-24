//! Posts affichés en cartes, et faux posts de démonstration.
//!
//! Les posts réels viennent de l'API (`api.rs`). Les faux posts servent
//! sans compte (builds de développement) : noms, comptes et textes
//! reprennent ceux des maquettes (`docs/mockups/source/Cartes.dc.html` et
//! `Colonnes.dc.html`). Ils sont fictifs.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    Following,
    Discover,
    LinuxEtLibre,
}

#[derive(Debug, Clone)]
pub struct Post {
    /// Adresse `at://` du post ; `None` pour les faux posts.
    pub uri: Option<String>,
    /// Identifiant du contenu (CID), nécessaire pour aimer ou reposter.
    pub cid: Option<String>,
    /// Mon « j'aime » sur ce post (adresse de l'enregistrement), s'il existe.
    pub like_uri: Option<String>,
    /// Mon repost de ce post, s'il existe.
    pub repost_uri: Option<String>,
    /// Post dans mes signets.
    pub bookmarked: bool,
    /// Premier post de la discussion, si ce post est une réponse.
    pub reply_root: Option<StrongRef>,
    /// Contexte affiché au-dessus du nom : notification (« a répondu à
    /// votre post ») ou repost (« Nadia Safi a reposté »).
    pub context: Option<String>,
    pub name: String,
    pub handle: String,
    /// Heure relative affichée sur la carte.
    pub time: String,
    /// Date complète affichée dans le volet de discussion.
    pub date: String,
    pub text: String,
    /// Liens, mentions et mots-dièse du texte.
    pub facets: Vec<crate::richtext::Facet>,
    /// Langues déclarées par l'auteur (« fr », « en-US »…).
    pub langs: Vec<String>,
    /// Adresse de l'avatar de l'auteur (miniature).
    pub avatar: Option<String>,
    /// Image ou vidéo jointe.
    pub media: Option<Media>,
    pub replies: u32,
    pub reposts: u32,
    pub likes: u32,
    pub thread: Vec<Reply>,
    /// Notification pas encore vue.
    pub unread: bool,
    /// Étiquettes de contenu du post et de son auteur (« graphic-media »,
    /// « porn », « !hide »…).
    pub labels: Vec<String>,
    /// Post épinglé en haut du profil de son auteur.
    pub pinned: bool,
    /// Post cité. Sans adresse : supprimé ou invisible, avec la raison
    /// dans le texte.
    pub quote: Option<Box<Post>>,
    /// Lien joint, avec son aperçu.
    pub link: Option<Link>,
    /// Notification regroupée : les personnes, de la plus récente à la plus
    /// ancienne, et la clé du groupe (raison, post concerné).
    pub group: Vec<Person>,
    pub group_key: Option<(String, String)>,
}

/// Personne citée dans une notification regroupée.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub handle: String,
    pub avatar: Option<String>,
}

/// Aperçu d'un lien joint à un post (embed `external`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Link {
    pub uri: String,
    pub title: String,
    pub description: String,
    pub thumb: Option<String>,
}

impl Link {
    /// Domaine affiché sur la carte : « exemple.org ».
    pub fn domain(&self) -> String {
        let rest = self
            .uri
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or(&self.uri);
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        host.strip_prefix("www.").unwrap_or(host).to_owned()
    }
}

impl Post {
    /// Post vide, sans auteur : base des posts cités ou introuvables.
    pub fn unavailable() -> Self {
        Post {
            uri: None,
            cid: None,
            like_uri: None,
            repost_uri: None,
            bookmarked: false,
            reply_root: None,
            context: None,
            name: String::new(),
            handle: String::new(),
            time: String::new(),
            date: String::new(),
            text: String::new(),
            facets: Vec::new(),
            langs: Vec::new(),
            avatar: None,
            media: None,
            replies: 0,
            reposts: 0,
            likes: 0,
            thread: Vec::new(),
            unread: false,
            labels: Vec::new(),
            pinned: false,
            quote: None,
            link: None,
            group: Vec::new(),
            group_key: None,
        }
    }
}

/// Liste de comptes (curation ou modération).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct List {
    pub uri: String,
    pub name: String,
    pub description: String,
    pub avatar: Option<String>,
    pub members: u64,
    /// Liste de modération (comptes masqués ou bloqués), sans fil de posts.
    pub moderation: bool,
    /// DID de la personne qui l'a créée.
    pub creator: String,
    /// Je masque les comptes de cette liste de modération.
    pub muted: bool,
    /// Mon blocage par cette liste (adresse de l'enregistrement), s'il existe.
    pub blocked: Option<String>,
}

pub fn demo_lists() -> Vec<List> {
    vec![
        List {
            uri: "demo:gnome".into(),
            name: "Communauté GNOME francophone".into(),
            description: "Contributrices et contributeurs de GNOME en français.".into(),
            avatar: None,
            members: 24,
            moderation: false,
            ..List::default()
        },
        List {
            uri: "demo:indesirables".into(),
            name: "Comptes indésirables".into(),
            description: "Pourriels et faux comptes.".into(),
            avatar: None,
            members: 3,
            moderation: true,
            ..List::default()
        },
    ]
}

/// Compte dans une liste de personnes (suggestions, résultats de recherche).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Actor {
    pub did: String,
    pub handle: String,
    /// Nom affiché, ou l'identifiant s'il n'y en a pas.
    pub name: String,
    pub bio: String,
    pub avatar: Option<String>,
    /// Mon abonnement à ce compte (adresse de l'enregistrement), s'il existe.
    pub following: Option<String>,
}

/// Sujet du moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Topic {
    /// Nom affiché.
    pub name: String,
    /// Recherche correspondante.
    pub query: String,
    /// Catégorie (`sports`, `politics`…), pour les tendances.
    pub category: Option<String>,
    /// Nombre de posts, s'il est connu.
    pub posts: u64,
    /// Tendance en forte hausse.
    pub hot: bool,
}

/// Personnes à suivre et sujets du moment de démonstration, repris de la
/// maquette `Explorer.dc.html`.
pub fn demo_topics() -> Vec<Topic> {
    [
        ("#GNOME49", "Logiciels libres", 1240, true),
        ("Accessibilité numérique", "Techno", 860, false),
        ("Photographie de montagne", "Photo", 540, false),
        ("Flatpak", "Logiciels libres", 410, false),
        ("Fête de la science", "Actualité", 320, false),
    ]
    .into_iter()
    .map(|(name, category, posts, hot)| Topic {
        name: name.to_owned(),
        query: name.to_owned(),
        category: Some(category.to_owned()),
        posts,
        hot,
    })
    .collect()
}

pub fn demo_actors() -> Vec<Actor> {
    [
        (
            "Nadia Safi",
            "nadia.example.social",
            "Traductrice, communauté GNOME fr",
        ),
        (
            "Romain Bailly",
            "romain.example.social",
            "Packaging Flatpak",
        ),
        (
            "Pierre Delmas",
            "pierre.example.social",
            "Accessibilité GTK",
        ),
    ]
    .into_iter()
    .map(|(name, handle, bio)| Actor {
        handle: handle.to_owned(),
        name: name.to_owned(),
        bio: bio.to_owned(),
        ..Default::default()
    })
    .collect()
}

/// Profil d'un compte.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    pub did: String,
    pub handle: String,
    /// Nom affiché, ou l'identifiant s'il n'y en a pas.
    pub name: String,
    pub bio: String,
    pub avatar: Option<String>,
    pub banner: Option<String>,
    pub followers: u64,
    pub follows: u64,
    pub posts: u64,
    /// Mon abonnement à ce compte (adresse de l'enregistrement), s'il existe.
    pub following: Option<String>,
    /// Ce compte me suit.
    pub follows_me: bool,
    /// Je masque ce compte.
    pub muted: bool,
    /// Mon blocage de ce compte (adresse de l'enregistrement), s'il existe.
    pub blocking: Option<String>,
    /// Comptes que je suis qui le suivent : quelques noms, et leur nombre.
    pub known_followers: Vec<String>,
    pub known_count: u64,
    /// Notifications d'activité demandées : ses posts, ses réponses.
    pub activity: (bool, bool),
    /// Nombre de flux, de listes et de starter packs créés par le compte.
    pub associated: (u64, u64, u64),
    /// Le compte est un service d'étiquetage.
    pub labeler: bool,
    /// Étiquettes du compte (comme celles des posts).
    pub labels: Vec<String>,
}

/// Référence à un post : son adresse et son contenu (CID).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrongRef {
    pub uri: String,
    pub cid: String,
}

/// Image (ou vignette de vidéo) jointe à un post.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Media {
    /// Texte alternatif, vide s'il n'y en a pas.
    pub alt: String,
    /// Adresse de la miniature ; `None` pour les faux posts.
    pub thumb: Option<String>,
    /// Largeur et hauteur, pour réserver la bonne place.
    pub aspect_ratio: Option<(u32, u32)>,
    /// Autres images du même post, non affichées.
    pub more: u32,
    /// Vidéo : adresse de sa liste de lecture (HLS), lue dans la carte.
    pub video: Option<String>,
    /// Vidéo à montrer comme un GIF : en boucle, sans son ni commandes.
    pub gif: bool,
    /// Toutes les images du post, pour la visionneuse.
    pub images: Vec<Image>,
}

/// Image d'un post : miniature, image en grand, texte alternatif.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Image {
    pub thumb: String,
    pub fullsize: String,
    pub alt: String,
}

impl Media {
    pub fn with_alt(alt: &str) -> Self {
        Self {
            alt: alt.to_owned(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub avatar: Option<String>,
    pub name: String,
    pub handle: String,
    pub time: String,
    pub text: String,
    pub facets: Vec<crate::richtext::Facet>,
}

#[allow(clippy::too_many_arguments)]
fn post(
    name: &'static str,
    handle: &'static str,
    time: &'static str,
    date: &'static str,
    text: &'static str,
    replies: u32,
    reposts: u32,
    likes: u32,
) -> Post {
    Post {
        uri: None,
        cid: None,
        like_uri: None,
        repost_uri: None,
        bookmarked: false,
        reply_root: None,
        context: None,
        name: name.into(),
        handle: handle.into(),
        time: demo_moment(time, date).0,
        date: demo_moment(time, date).1,
        text: text.into(),
        // Les mentions de démonstration ouvrent le profil par identifiant.
        facets: crate::richtext::detect(text),
        langs: vec!["fr".into()],
        avatar: None,
        media: None,
        replies,
        reposts,
        likes,
        thread: Vec::new(),
        unread: false,
        labels: Vec::new(),
        pinned: false,
        quote: None,
        link: None,
        group: Vec::new(),
        group_key: None,
    }
}

fn reply(
    name: &'static str,
    handle: &'static str,
    time: &'static str,
    text: &'static str,
) -> Reply {
    Reply {
        avatar: None,
        name: name.into(),
        handle: handle.into(),
        time: demo_moment(time, "").0,
        text: text.into(),
        facets: crate::richtext::detect(text),
    }
}

/// Instant de référence des données de démonstration : le 20 novembre 2026 à 14:59, heure locale.
pub fn demo_now() -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone;
    chrono::Local
        .with_ymd_and_hms(2026, 11, 20, 14, 59, 0)
        .earliest()
        .unwrap_or_else(chrono::Local::now)
}

/// Heure relative et date complète d'un post de démonstration, dans la langue de l'interface.
/// Les données les donnent en français : « 4 min » ou « 2 h », et « 20 nov., 14:55 » (date
/// vide : aucune). La date, si elle est donnée, fait foi.
fn demo_moment(time: &str, date: &str) -> (String, String) {
    use chrono::TimeZone;
    let now = demo_now();
    let from_date = date.split_once(" nov., ").and_then(|(day, clock)| {
        let (hour, minute) = clock.split_once(':')?;
        chrono::Local
            .with_ymd_and_hms(
                2026,
                11,
                day.parse().ok()?,
                hour.parse().ok()?,
                minute.parse().ok()?,
                0,
            )
            .earliest()
    });
    let from_time = || {
        let (count, unit) = time.split_once(' ')?;
        let count: i64 = count.parse().ok()?;
        let elapsed = match unit {
            "min" => chrono::Duration::minutes(count),
            "h" => chrono::Duration::hours(count),
            "j" => chrono::Duration::days(count),
            _ => return None,
        };
        Some(now - elapsed)
    };
    let Some(then) = from_date.or_else(from_time) else {
        return (time.to_owned(), date.to_owned());
    };
    let then = then.fixed_offset();
    let relative = crate::api::relative_time(then, now.to_utc());
    let full = if date.is_empty() {
        String::new()
    } else {
        crate::api::full_date(then)
    };
    (relative, full)
}

fn lucie_gnome_49() -> Post {
    const TEXT: &str = "Notes de version de GNOME 49 : le nouveau sélecteur de fichiers est \
                        enfin utilisable au clavier de bout en bout. Merci à celles et ceux \
                        qui ont testé la bêta, dont @aya.example.social. #GNOME49";
    Post {
        text: TEXT.into(),
        facets: crate::richtext::detect(TEXT),
        thread: vec![
            reply(
                "Aya Koné",
                "aya.example.social",
                "20 min",
                "Super nouvelle. Le raccourci pour ouvrir le dossier parent est-il \
                 documenté quelque part ?",
            ),
            reply(
                "Romain Bailly",
                "romain.example.social",
                "12 min",
                "Je confirme, testé hier sur Fedora. Mention spéciale pour la recherche \
                 récursive.",
            ),
            reply(
                "Nadia Safi",
                "nadia.example.social",
                "5 min",
                "Et la vue en grille des miniatures, c'est pour 50 ou plus tard ?",
            ),
        ],
        ..post(
            "Lucie Morel",
            "lucie.example.social",
            "27 min",
            "20 nov., 14:32",
            "",
            8,
            12,
            48,
        )
    }
}

pub fn posts(feed: Feed) -> Vec<Post> {
    match feed {
        Feed::Following => vec![
            post(
                "Romain Bailly",
                "romain.example.social",
                "4 min",
                "20 nov., 14:55",
                "Premier essai de Flathub en mode hors ligne dans le train : tout \
                 fonctionne, sauf la météo de mon appli de météo.",
                5,
                2,
                31,
            ),
            lucie_gnome_49(),
            Post {
                media: Some(Media::with_alt(
                    "Lever de soleil sur les falaises du Vercors, ciel orange au-dessus \
                     d'une mer de nuages.",
                )),
                thread: vec![reply(
                    "Pierre Delmas",
                    "pierre.example.social",
                    "40 min",
                    "Magnifique. Tu étais à quel belvédère ?",
                )],
                ..post(
                    "Aya Koné",
                    "aya.example.social",
                    "1 h",
                    "20 nov., 13:58",
                    "Lever de soleil sur le Vercors ce matin, 6 h 40, moins 3 degrés.",
                    14,
                    9,
                    212,
                )
            },
            // Post avec un lien joint.
            Post {
                link: Some(Link {
                    uri: "https://www.example.org/blog/accessibilite-gtk4".into(),
                    title: "Ce que Orca lit vraiment dans une application GTK4".into(),
                    description: "Noms accessibles, rôles, états : un tour des \
                                  applications GNOME avec un lecteur d'écran."
                        .into(),
                    thumb: None,
                }),
                ..post(
                    "Pierre Delmas",
                    "pierre.example.social",
                    "2 h",
                    "20 nov., 12:47",
                    "Un billet sur l'accessibilité des applications GTK4 : ce que Orca lit \
                     vraiment.",
                    3,
                    6,
                    57,
                )
            },
            // Post étiqueté : son image est floutée sous un avertissement.
            Post {
                media: Some(Media::with_alt(
                    "Plaie suturée sur la patte d'un chien, juste après l'intervention.",
                )),
                labels: vec!["graphic-media".into()],
                ..post(
                    "Inès Garnier",
                    "ines.example.social",
                    "2 h",
                    "20 nov., 12:20",
                    "Dernier jour de stage à la clinique vétérinaire : Filou est reparti \
                     avec six points de suture et beaucoup de friandises.",
                    4,
                    1,
                    63,
                )
            },
            // Citation d'un autre post.
            Post {
                quote: Some(Box::new(lucie_gnome_49())),
                ..post(
                    "Nadia Safi",
                    "nadia.example.social",
                    "3 h",
                    "20 nov., 11:40",
                    "Et la traduction française est complète, glossaire à jour compris.",
                    2,
                    4,
                    39,
                )
            },
            post(
                "Romain Bailly",
                "romain.example.social",
                "5 h",
                "20 nov., 09:31",
                "Qui a testé le portail de sélection de fichiers sous Wayland avec un \
                 lecteur d'écran ?",
                9,
                3,
                22,
            ),
        ],
        Feed::Discover => vec![
            post(
                "Marc Collin",
                "marc.example.social",
                "8 min",
                "20 nov., 14:51",
                "Dix ans de Flatpak : ce que la distribution d'applications a changé pour \
                 les bureaux Linux.",
                21,
                40,
                310,
            ),
            post(
                "Sophie Berger",
                "sophie.example.social",
                "21 min",
                "20 nov., 14:38",
                "Atelier d'écriture accessible ce samedi à la médiathèque, inscriptions \
                 ouvertes.",
                4,
                12,
                88,
            ),
            Post {
                media: Some(Media::with_alt(
                    "Carnet ouvert posé sur un rocher, paysage de lave en fond.",
                )),
                ..post(
                    "Clara Giraud",
                    "clara.example.social",
                    "1 h",
                    "20 nov., 13:50",
                    "Carnet de voyage sonore : trois semaines en Islande, 40 enregistrements \
                     de terrain.",
                    11,
                    25,
                    420,
                )
            },
            post(
                "Marc Collin",
                "marc.example.social",
                "2 h",
                "20 nov., 12:40",
                "Petit rappel : sauvegardez avant de mettre à jour, même quand tout se \
                 passe bien.",
                7,
                31,
                260,
            ),
        ],
        Feed::LinuxEtLibre => vec![
            post(
                "Hugo Renard",
                "hugo.example.social",
                "12 min",
                "20 nov., 14:47",
                "Sortie de la version 1.8 : meilleure prise en charge des écrans à haute \
                 densité.",
                6,
                14,
                97,
            ),
            post(
                "Inès Martin",
                "ines.example.social",
                "30 min",
                "20 nov., 14:29",
                "Bureau GNOME en 4K à 100 % : on gagne énormément de place, mais le texte \
                 devient petit.",
                18,
                7,
                156,
            ),
            lucie_gnome_49(),
            post(
                "Fanny Dumas",
                "fanny.example.social",
                "1 h",
                "20 nov., 13:45",
                "Rapport de la rencontre des traducteurs : 14 langues, un seul dépôt.",
                3,
                8,
                72,
            ),
        ],
    }
}

/// Notifications : posts précédés d'un contexte, comme dans la maquette
/// `Colonnes.dc.html`.
/// Notifications de démonstration qui sont des mentions, réponses ou
/// citations.
pub fn demo_mentions() -> Vec<Post> {
    let kept = [
        gettextrs::gettext("a répondu à votre post"),
        gettextrs::gettext("vous a mentionné"),
        gettextrs::gettext("a cité votre post"),
    ];
    notifications()
        .into_iter()
        .filter(|post| post.context.as_ref().is_some_and(|c| kept.contains(c)))
        .collect()
}

pub fn notifications() -> Vec<Post> {
    // Les deux plus récentes ne sont pas encore vues.
    let mut unread = 2;
    let mut notification = |context, post: Post| {
        let is_unread = unread > 0;
        unread -= 1;
        Post {
            context: Some(gettextrs::gettext(context)),
            unread: is_unread,
            ..post
        }
    };
    vec![
        notification(
            "a répondu à votre post",
            post(
                "Aya Koné",
                "aya.example.social",
                "2 min",
                "20 nov., 14:57",
                "Super nouvelle. Le raccourci pour ouvrir le dossier parent est-il \
                 documenté ?",
                0,
                0,
                3,
            ),
        ),
        notification(
            "vous a mentionné",
            post(
                "Lucie Morel",
                "lucie.example.social",
                "18 min",
                "20 nov., 14:41",
                "Test de la bêta de GNOME 49 sur mon portable : le sélecteur de fichiers, \
                 c'est du bon.",
                2,
                1,
                12,
            ),
        ),
        Post {
            group: [
                ("Pierre Delmas", "pierre.example.social"),
                ("Nadia Safi", "nadia.example.social"),
                ("Romain Bailly", "romain.example.social"),
                ("Lucie Morel", "lucie.example.social"),
                ("Aya Koné", "aya.example.social"),
            ]
            .into_iter()
            .map(|(name, handle)| Person {
                name: name.into(),
                handle: handle.into(),
                avatar: None,
            })
            .collect(),
            ..notification(
                "Pierre Delmas et 4 autres ont aimé votre post",
                post(
                    "Pierre Delmas",
                    "pierre.example.social",
                    "40 min",
                    "20 nov., 14:19",
                    "Rappel : ajoutez un texte alternatif à vos captures d'écran.",
                    1,
                    5,
                    20,
                ),
            )
        },
        notification(
            "a répondu à votre post",
            post(
                "Romain Bailly",
                "romain.example.social",
                "2 h",
                "20 nov., 12:59",
                "Merci pour la capture, elle m'a bien aidé.",
                0,
                0,
                4,
            ),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_domains_drop_scheme_path_and_www() {
        let link = |uri: &str| Link {
            uri: uri.into(),
            ..Default::default()
        };
        assert_eq!(
            link("https://www.example.org/blog/a?b=1").domain(),
            "example.org"
        );
        assert_eq!(link("http://gnome.org").domain(), "gnome.org");
        assert_eq!(link("https://docs.gtk.org#top").domain(), "docs.gtk.org");
    }

    #[test]
    fn demo_mentions_keep_replies_and_mentions_only() {
        let all = notifications();
        let mentions = demo_mentions();
        assert!(!mentions.is_empty());
        assert!(mentions.len() < all.len());
        assert!(mentions.iter().all(|post| {
            let context = post.context.as_deref().unwrap_or_default();
            !context.contains("aimé")
        }));
    }
}
