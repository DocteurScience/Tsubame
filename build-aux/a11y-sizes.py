#!/usr/bin/env python3
"""Cibles tactiles de Tsubame sous 44 px, et éléments au-delà de 360 px de large (AT-SPI).
Usage, fenêtre réduite à 360 px :  python3 build-aux/a11y-sizes.py "écran"
"""
import pyatspi, sys
label=sys.argv[1]; MIN=44
d=pyatspi.Registry.getDesktop(0)
app=[d.getChildAtIndex(i) for i in range(d.childCount) if 'tsubame' in (d.getChildAtIndex(i).name or '').lower()][0]
ROLES={pyatspi.ROLE_PUSH_BUTTON,pyatspi.ROLE_TOGGLE_BUTTON,pyatspi.ROLE_CHECK_BOX,pyatspi.ROLE_RADIO_BUTTON,pyatspi.ROLE_LINK}
small=[]; total=0; offscreen=0
def walk(o,depth=0):
    global total, offscreen
    if o is None or depth>70: return
    try:
        st=o.getState()
        if depth>0 and o.getRole()!=pyatspi.ROLE_FRAME and not st.contains(pyatspi.STATE_SHOWING): return
        if o.getRole() in ROLES and st.contains(pyatspi.STATE_SENSITIVE):
            e=o.queryComponent().getExtents(pyatspi.WINDOW_COORDS)
            if e.width>0 and e.height>0:
                total+=1
                if e.x+e.width>362: offscreen+=1
                if e.width<MIN or e.height<MIN:
                    small.append(f"{o.getRoleName()} «{(o.name or '')[:40]}» {e.width}×{e.height}")
        for i in range(o.childCount): walk(o.getChildAtIndex(i),depth+1)
    except Exception: pass
walk(app)
print(f"== {label} : {total} cibles, {len(small)} sous {MIN} px, {offscreen} hors écran")
for s in dict.fromkeys(small): print('  ', s)
