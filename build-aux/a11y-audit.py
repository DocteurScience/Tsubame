#!/usr/bin/env python3
"""Audit d'accessibilité de Tsubame en cours d'exécution, par AT-SPI (ce que voit Orca).

Signale les éléments interactifs visibles sans nom, ceux qu'on ne peut pas atteindre au
clavier, et les images sans nom (à marquer décoratives sinon). Usage, Tsubame ouvert :
    python3 build-aux/a11y-audit.py "nom de l'écran"
Demande pyatspi (paquet python3-pyatspi) et le bus d'accessibilité (at-spi2-core).
"""
import sys, pyatspi
label = sys.argv[1] if len(sys.argv) > 1 else ''
desktop = pyatspi.Registry.getDesktop(0)
app = None
for i in range(desktop.childCount):
    a = desktop.getChildAtIndex(i)
    if a and a.name and 'tsubame' in a.name.lower():
        app = a
if app is None:
    print('application introuvable'); sys.exit(1)
INTERACTIVE = {pyatspi.ROLE_PUSH_BUTTON, pyatspi.ROLE_TOGGLE_BUTTON, pyatspi.ROLE_CHECK_BOX,
               pyatspi.ROLE_RADIO_BUTTON, pyatspi.ROLE_LINK, pyatspi.ROLE_MENU_ITEM, pyatspi.ROLE_SWITCH if hasattr(pyatspi,'ROLE_SWITCH') else -1,
               pyatspi.ROLE_ENTRY, pyatspi.ROLE_TEXT, pyatspi.ROLE_COMBO_BOX, pyatspi.ROLE_SPIN_BUTTON}
problems = []; count = 0
def path(o):
    parts = []
    p = o
    for _ in range(4):
        p = p.parent
        if p is None: break
        if p.name: parts.append(f"{p.getRoleName()}«{p.name[:30]}»")
    return ' < '.join(parts)
def focusable_inside(o, depth=0):
    if depth > 4: return False
    for i in range(o.childCount):
        c = o.getChildAtIndex(i)
        if c is None: continue
        if c.getState().contains(pyatspi.STATE_FOCUSABLE) or focusable_inside(c, depth+1):
            return True
    return False
def details(o):
    p = o.parent
    sib = [p.getChildAtIndex(i) for i in range(p.childCount)] if p else []
    idx = next((i for i,c in enumerate(sib) if c == o), -1)
    names = [f"{c.getRoleName()}«{(c.name or '')[:25]}»" for c in sib]
    return f"parent={p.getRoleName() if p else '-'}«{(p.name or '')[:25] if p else ''}» rang={idx} desc={o.description!r} frères={names}"
def walk(o, depth=0):
    global count
    if o is None or depth > 60: return
    try:
        st = o.getState()
        if depth > 0 and o.getRole() != pyatspi.ROLE_FRAME and not st.contains(pyatspi.STATE_SHOWING):
            return
        role = o.getRole()
        if role in INTERACTIVE:
            count += 1
            name = (o.name or '').strip()
            if not name and role not in (pyatspi.ROLE_TEXT, pyatspi.ROLE_ENTRY):
                problems.append(f"SANS NOM  {o.getRoleName()}  {details(o)}")
            if not st.contains(pyatspi.STATE_FOCUSABLE) and role not in (pyatspi.ROLE_MENU_ITEM,) and st.contains(pyatspi.STATE_SENSITIVE) and not focusable_inside(o):
                problems.append(f"PAS FOCUS {o.getRoleName()} «{name[:40]}» dans {path(o)}")
        if role == pyatspi.ROLE_IMAGE and st.contains(pyatspi.STATE_SHOWING) and not (o.name or '').strip():
            problems.append(f"IMAGE SANS NOM {details(o)}")
        for i in range(o.childCount):
            walk(o.getChildAtIndex(i), depth+1)
    except Exception as e:
        pass
walk(app)
print(f"== {label} : {count} éléments interactifs visibles, {len(problems)} problèmes")
seen = set()
for p in problems:
    if p not in seen:
        seen.add(p); print('  ', p)
