//! Faux posts pour développer l'interface sans connexion réseau.
//!
//! Les noms, comptes et textes reprennent ceux des maquettes
//! (`docs/mockups/source/Cartes.dc.html` et `Colonnes.dc.html`). Ils sont
//! fictifs.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    Following,
    Discover,
    LinuxEtLibre,
}

#[derive(Debug, Clone)]
pub struct Post {
    pub name: &'static str,
    pub handle: &'static str,
    /// Heure relative affichée sur la carte.
    pub time: &'static str,
    /// Date complète affichée dans le volet de discussion.
    pub date: &'static str,
    pub text: &'static str,
    /// Texte alternatif de l'image jointe, s'il y en a une.
    pub image_alt: Option<&'static str>,
    pub replies: u32,
    pub reposts: u32,
    pub likes: u32,
    pub thread: Vec<Reply>,
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub name: &'static str,
    pub handle: &'static str,
    pub time: &'static str,
    pub text: &'static str,
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
        name,
        handle,
        time,
        date,
        text,
        image_alt: None,
        replies,
        reposts,
        likes,
        thread: Vec::new(),
    }
}

fn reply(
    name: &'static str,
    handle: &'static str,
    time: &'static str,
    text: &'static str,
) -> Reply {
    Reply {
        name,
        handle,
        time,
        text,
    }
}

fn lucie_gnome_49() -> Post {
    Post {
        text: "Notes de version de GNOME 49 : le nouveau sélecteur de fichiers est enfin \
               utilisable au clavier de bout en bout. Merci à celles et ceux qui ont testé \
               la bêta.",
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
                image_alt: Some(
                    "Lever de soleil sur les falaises du Vercors, ciel orange au-dessus \
                     d'une mer de nuages.",
                ),
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
                image_alt: Some("Carnet ouvert posé sur un rocher, paysage de lave en fond."),
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
