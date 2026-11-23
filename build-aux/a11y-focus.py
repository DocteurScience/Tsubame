#!/usr/bin/env python3
"""Nom de l'élément qui a le focus clavier dans Tsubame (AT-SPI). Usage, après chaque Tab :
    python3 build-aux/a11y-focus.py "étape"
"""
import pyatspi, sys
d=pyatspi.Registry.getDesktop(0)
app=[d.getChildAtIndex(i) for i in range(d.childCount) if 'tsubame' in (d.getChildAtIndex(i).name or '').lower()][0]
found=[]
def walk(o,depth=0):
    if o is None or depth>70 or found: return
    try:
        if o.getState().contains(pyatspi.STATE_FOCUSED):
            found.append(f"{o.getRoleName()} «{(o.name or '')[:50]}»"); return
        for i in range(o.childCount): walk(o.getChildAtIndex(i),depth+1)
    except Exception: pass
walk(app)
print(sys.argv[1], '→', found[0] if found else '(rien)')
