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
}

/// Liste de comptes (curation ou modération).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct List {
    pub uri: String,
    pub name: String,
    pub description: String,
    pub avatar: Option<String>,
    pub members: u64,
    /// Liste de modération (comptes masqués ou bloqués), sans fil de posts.
    pub moderation: bool,
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
        },
        List {
            uri: "demo:indesirables".into(),
            name: "Comptes indésirables".into(),
            description: "Pourriels et faux comptes.".into(),
            avatar: None,
            members: 3,
            moderation: true,
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
}

/// Personnes à suivre et sujets du moment de démonstration, repris de la
/// maquette `Explorer.dc.html`.
pub fn demo_topics() -> Vec<Topic> {
    [
        "#GNOME49",
        "Accessibilité numérique",
        "Photographie de montagne",
    ]
    .into_iter()
    .map(|name| Topic {
        name: name.to_owned(),
        query: name.to_owned(),
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
        time: time.into(),
        date: date.into(),
        text: text.into(),
        avatar: None,
        media: None,
        replies,
        reposts,
        likes,
        thread: Vec::new(),
        unread: false,
        labels: Vec::new(),
        pinned: false,
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
        time: time.into(),
        text: text.into(),
    }
}

fn lucie_gnome_49() -> Post {
    Post {
        text: "Notes de version de GNOME 49 : le nouveau sélecteur de fichiers est enfin \
               utilisable au clavier de bout en bout. Merci à celles et ceux qui ont testé \
               la bêta."
            .into(),
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
            post(
                "Pierre Delmas",
                "pierre.example.social",
                "2 h",
                "20 nov., 12:47",
                "Un billet sur l'accessibilité des applications GTK4 : ce que Orca lit \
                 vraiment.",
                3,
                6,
                57,
            ),
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
            post(
                "Nadia Safi",
                "nadia.example.social",
                "3 h",
                "20 nov., 11:40",
                "Le glossaire de traduction est à jour : une cinquantaine de termes \
                 harmonisés cette semaine.",
                2,
                4,
                39,
            ),
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
        notification(
            "a aimé et reposté votre post",
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
        ),
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
