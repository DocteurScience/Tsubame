#!/usr/bin/env python3
"""Vérification d'accessibilité de Tsubame, lancée par a11y-check.sh.

Tsubame tourne en démonstration sous Xvfb, avec le bus d'accessibilité. Le
script ouvre les écrans principaux en activant les boutons par leur nom
(AT-SPI), puis vérifie, comme Orca les voit :
- que chaque élément interactif visible a un nom et s'atteint au clavier ;
- qu'aucune image n'est sans nom (sinon, la marquer décorative) ;
- en palier mobile (360 px), que rien ne dépasse à droite et que les
  cibles font au moins 44 px, hors exceptions choisies (voir ALLOWED).
Sortie non nulle s'il reste un problème.
"""

import subprocess
import sys
import time

import pyatspi

MIN_TARGET = 44
# Exceptions acceptées en palier mobile (voir docs/design.md, Accessibilité).
ALLOWED_SMALL = (
    "Voir le profil de",  # nom d'auteur, lien dans le texte ; l'avatar fait 44 px
    "Minimize",
    "Maximize",
    "Close",
    "Ajouter une colonne",  # barre latérale, masquée en mobile
)
INTERACTIVE = {
    pyatspi.ROLE_PUSH_BUTTON,
    pyatspi.ROLE_TOGGLE_BUTTON,
    pyatspi.ROLE_CHECK_BOX,
    pyatspi.ROLE_RADIO_BUTTON,
    pyatspi.ROLE_LINK,
    pyatspi.ROLE_ENTRY,
    pyatspi.ROLE_TEXT,
    pyatspi.ROLE_COMBO_BOX,
}
TARGETS = {
    pyatspi.ROLE_PUSH_BUTTON,
    pyatspi.ROLE_TOGGLE_BUTTON,
    pyatspi.ROLE_RADIO_BUTTON,
    pyatspi.ROLE_LINK,
}

failures = []


def app():
    for _ in range(60):
        desktop = pyatspi.Registry.getDesktop(0)
        for i in range(desktop.childCount):
            a = desktop.getChildAtIndex(i)
            if a is not None and "tsubame" in (a.name or "").lower():
                return a
        time.sleep(0.5)
    sys.exit("Tsubame introuvable sur le bus d'accessibilité")


def visible(o, depth):
    st = o.getState()
    return depth == 0 or o.getRole() == pyatspi.ROLE_FRAME or st.contains(pyatspi.STATE_SHOWING)


def walk(o, f, depth=0):
    if o is None or depth > 70:
        return
    try:
        if not visible(o, depth):
            return
        f(o, depth)
        children = [o.getChildAtIndex(i) for i in range(o.childCount)]
    except Exception:
        return
    for c in children:
        walk(c, f, depth + 1)


def focusable_inside(o, depth=0):
    if depth > 4:
        return False
    for i in range(o.childCount):
        c = o.getChildAtIndex(i)
        if c is not None and (c.getState().contains(pyatspi.STATE_FOCUSABLE) or focusable_inside(c, depth + 1)):
            return True
    return False


def find(name, role=None):
    found = []

    def f(o, depth):
        if found or (o.name or "") != name or (role is not None and o.getRole() != role):
            return
        try:
            if o.queryAction().nActions > 0:
                found.append(o)
        except NotImplementedError:
            pass

    walk(app(), f)
    return found[0] if found else None


def activate(name):
    for _ in range(20):
        o = find(name)
        if o is not None:
            action = o.queryAction()
            action.doAction(0)
            time.sleep(1.5)
            return
        time.sleep(0.5)
    failures.append(f"bouton « {name} » introuvable")


def key(*keys):
    subprocess.run(["xdotool", "key", *keys], check=False)
    time.sleep(1.2)


def resize(width, height):
    window = subprocess.run(
        ["xdotool", "search", "--onlyvisible", "--name", "Tsubame"],
        capture_output=True, text=True,
    ).stdout.split()
    if window:
        subprocess.run(["xdotool", "windowsize", window[-1], str(width), str(height)], check=False)
    time.sleep(2.5)


def check_names(screen):
    problems = []

    def f(o, depth):
        role = o.getRole()
        st = o.getState()
        name = (o.name or "").strip()
        if role in INTERACTIVE:
            if not name and role not in (pyatspi.ROLE_TEXT, pyatspi.ROLE_ENTRY):
                problems.append(f"{o.getRoleName()} sans nom (description : {o.description!r})")
            if (
                st.contains(pyatspi.STATE_SENSITIVE)
                and not st.contains(pyatspi.STATE_FOCUSABLE)
                and not focusable_inside(o)
            ):
                problems.append(f"{o.getRoleName()} « {name[:40]} » hors du clavier")
        if role == pyatspi.ROLE_IMAGE and not name:
            problems.append(f"image sans nom (description : {o.description!r})")

    walk(app(), f)
    report(screen, problems)


def check_sizes(screen, width=360):
    problems = []

    def f(o, depth):
        if o.getRole() not in TARGETS or not o.getState().contains(pyatspi.STATE_SENSITIVE):
            return
        e = o.queryComponent().getExtents(pyatspi.WINDOW_COORDS)
        if e.width <= 0 or e.height <= 0:
            return
        name = o.name or ""
        if e.x + e.width > width + 2:
            problems.append(f"« {name[:40]} » dépasse à droite (x={e.x}, largeur {e.width})")
        if (e.width < MIN_TARGET or e.height < MIN_TARGET) and not name.startswith(ALLOWED_SMALL):
            problems.append(f"« {name[:40]} » {e.width}×{e.height} px")

    walk(app(), f)
    if any("dépasse" in p for p in problems):
        problems.append("conteneurs trop larges : " + ", ".join(widest(width)))
    report(screen + " (mobile)", problems)


def widest(width, limit=6):
    """Conteneurs qui dépassent `width`, les plus profonds d'abord : la
    piste de ce qui impose la largeur."""
    found = []

    def f(o, depth):
        try:
            e = o.queryComponent().getExtents(pyatspi.WINDOW_COORDS)
        except NotImplementedError:
            return
        if e.x + e.width > width + 2:
            found.append((depth, f"{o.getRoleName()}«{(o.name or '')[:20]}» {e.width}px"))

    walk(app(), f)
    return [name for _, name in sorted(found, reverse=True)[:limit]]


def report(screen, problems):
    problems = list(dict.fromkeys(problems))
    print(f"{screen} : {'aucun problème' if not problems else f'{len(problems)} problème(s)'}")
    for p in problems:
        print("   ", p)
    failures.extend(f"{screen} : {p}" for p in problems)


def main():
    app()
    resize(1000, 800)
    check_names("connexion")
    activate("Continuer avec les posts de démonstration")
    resize(1400, 900)
    check_names("accueil")
    key("ctrl+n")
    check_names("rédaction")
    key("Escape")
    key("ctrl+comma")
    check_names("préférences")
    key("Escape")
    for page in ("Explorer", "Messages", "Flux", "Signets", "Listes"):
        activate(page)
        check_names(page.lower())
    activate("Following")

    resize(360, 740)
    check_sizes("accueil")
    key("ctrl+n")
    check_sizes("rédaction")
    key("Escape")
    activate("Voir le profil de Romain Bailly")
    check_sizes("profil")

    if failures:
        print(f"\n{len(failures)} problème(s) d'accessibilité.")
        sys.exit(1)
    print("\nAccessibilité : aucun problème.")


if __name__ == "__main__":
    main()
