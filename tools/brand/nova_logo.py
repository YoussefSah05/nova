"""Nova logo generator: the N is Bricolage Grotesque's own N (wght 620), split into
three planes of rising light, with a stellar flare on the tip of the right stem."""
import json, math
# Bricolage Grotesque N at wght 620 (font units, y up, cap height 660, advance 676)
LEFT  = [(45.4,0),(174.5,0),(174.5,660),(45.4,660)]
DIAG  = [(85.9,660),(223.1,660),(594.6,0),(461.4,0)]
RIGHT = [(501.2,0),(630.6,0),(630.6,660),(501.2,660)]
CAP, ADV = 660, 676
STAR_X = (501.2+630.6)/2
OVA = json.load(open("ova.json"))

class Frame:
    """Place the N: font units -> canvas, with `h` = rendered cap height."""
    def __init__(self, ox, base, h):
        self.s = h/CAP; self.ox = ox; self.base = base
    def pt(self, x, y): return (self.ox + x*self.s, self.base - y*self.s)
    def poly(self, pts): return "M" + " L".join(f"{a:.1f},{b:.1f}" for a,b in (self.pt(*p) for p in pts)) + "Z"
    @property
    def star(self): return self.pt(STAR_X, CAP)

def spike(cx, cy, ang, length, half):
    a=math.radians(ang); ux,uy=math.cos(a),math.sin(a); px,py=-uy,ux
    tip=(cx+ux*length, cy+uy*length)
    c=lambda sgn:(cx+ux*length*0.28+sgn*px*half*0.18, cy+uy*length*0.28+sgn*py*half*0.18)
    b=lambda sgn:(cx+sgn*px*half, cy+sgn*py*half)
    (b1,b2,c1,c2)=(b(1),b(-1),c(1),c(-1))
    return f"M{b1[0]:.1f},{b1[1]:.1f} Q{c1[0]:.1f},{c1[1]:.1f} {tip[0]:.1f},{tip[1]:.1f} Q{c2[0]:.1f},{c2[1]:.1f} {b2[0]:.1f},{b2[1]:.1f}Z"

def star_shapes(cx, cy, k, small=False):
    """Diffraction-spike star; k scales with the letter. `small` = bolder rays for tiny sizes."""
    if small:
        longs=[spike(cx,cy,-90,150*k,34*k), spike(cx,cy,90,80*k,30*k), spike(cx,cy,0,120*k,30*k), spike(cx,cy,180,120*k,30*k)]
        return longs, [], 40*k
    longs=[spike(cx,cy,-90,168*k,20*k), spike(cx,cy,90,104*k,20*k), spike(cx,cy,0,124*k,18*k), spike(cx,cy,180,124*k,18*k)]
    shorts=[spike(cx,cy,a,62*k,11*k) for a in (-45,45,135,-135)]
    return longs, shorts, 46*k

PALETTES = {
  "dark":  dict(dim=("#8a561c","#b47a2c"), beam=("#b47a2c","#e0a33e"), bright=("#e0a33e","#ffd27e"),
                ray=("#fff6e0","#ffd98f","#f3b452"), hot=("#ffffff","#fff4d8","#ffd98f"), word="#f4ede2"),
  "light": dict(dim=("#8a561c","#a86f25"), beam=("#a86f25","#d0922f"), bright=("#d0922f","#e9ac45"),
                ray=("#f6c25e","#eaa63d","#d48f26"), hot=("#fbd27a","#f6c25e","#f0b240"), word="#141216"),
}

def defs(fr, pal, k, glow_r=None, bg_center=None):
    P=PALETTES[pal]; (lx,top)=fr.pt(45.4,CAP); (rx,bot)=fr.pt(630.6,0); (cx,cy)=fr.star
    g = f'''<linearGradient id="dim" x1="0" y1="{bot:.0f}" x2="0" y2="{top:.0f}" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="{P['dim'][0]}"/><stop offset="1" stop-color="{P['dim'][1]}"/></linearGradient>
<linearGradient id="beam" x1="{lx:.0f}" y1="{top:.0f}" x2="{rx:.0f}" y2="{bot:.0f}" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="{P['beam'][0]}"/><stop offset="1" stop-color="{P['beam'][1]}"/></linearGradient>
<linearGradient id="bright" x1="0" y1="{bot:.0f}" x2="0" y2="{top:.0f}" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="{P['bright'][0]}"/><stop offset="1" stop-color="{P['bright'][1]}"/></linearGradient>
<radialGradient id="ray" cx="{cx:.1f}" cy="{cy:.1f}" r="{168*k:.0f}" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="{P['ray'][0]}"/><stop offset="0.45" stop-color="{P['ray'][1]}"/><stop offset="1" stop-color="{P['ray'][2]}"/></radialGradient>
<radialGradient id="hot" cx="{cx:.1f}" cy="{cy:.1f}" r="{46*k:.0f}" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="{P['hot'][0]}"/><stop offset="0.45" stop-color="{P['hot'][1]}"/><stop offset="1" stop-color="{P['hot'][2]}" stop-opacity="0"/></radialGradient>'''
    if glow_r:
        g += f'<radialGradient id="glow" cx="{cx:.1f}" cy="{cy:.1f}" r="{glow_r}" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#ffc764" stop-opacity="0.42"/><stop offset="0.35" stop-color="#e0a33e" stop-opacity="0.13"/><stop offset="1" stop-color="#e0a33e" stop-opacity="0"/></radialGradient>'
    if bg_center:
        g += f'<radialGradient id="bg" cx="{cx:.1f}" cy="{cy:.1f}" r="820" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#1c150d"/><stop offset="0.5" stop-color="#0f0e11"/><stop offset="1" stop-color="#08080a"/></radialGradient>'
    return g

def mark(fr, k, mono=None, small=False):
    longs, shorts, core = star_shapes(*fr.star, k, small)
    cx, cy = fr.star
    if mono:
        f=lambda d: f'<path d="{d}" fill="{mono}"/>'
        return "".join(f(fr.poly(p)) for p in (LEFT,DIAG,RIGHT)) + "".join(f(d) for d in longs+shorts) + f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="{core*0.62:.1f}" fill="{mono}"/>'
    return (f'<path d="{fr.poly(LEFT)}" fill="url(#dim)"/><path d="{fr.poly(DIAG)}" fill="url(#beam)"/><path d="{fr.poly(RIGHT)}" fill="url(#bright)"/>'
            + "".join(f'<path d="{d}" fill="url(#ray)" opacity="0.7"/>' for d in shorts)
            + "".join(f'<path d="{d}" fill="url(#ray)"/>' for d in longs)
            + f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="{core:.1f}" fill="url(#hot)"/>')

def icon(small=False):
    h=400; k=h/392
    # bounding box of N + star (font units -> px) to centre optically
    s=h/CAP; width=(630.6-45.4)*s; star_up=168*k; star_right=124*k-(630.6-STAR_X)*s
    bw=width+max(0,star_right); bh=h+star_up
    ox=512-bw/2-45.4*s; base=528+bh/2
    fr=Frame(ox, base, h); cx,cy=fr.star
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024"><defs>{defs(fr,"dark",k,330,True)}'
            '<clipPath id="tile"><rect x="100" y="100" width="824" height="824" rx="185"/></clipPath></defs>'
            '<g clip-path="url(#tile)"><rect x="100" y="100" width="824" height="824" fill="url(#bg)"/>'
            f'<circle cx="{cx:.1f}" cy="{cy:.1f}" r="330" fill="url(#glow)"/></g>'
            '<rect x="101" y="101" width="822" height="822" rx="184" fill="none" stroke="#fff" stroke-opacity="0.07" stroke-width="2"/>'
            f'{mark(fr,k,small=small)}</svg>')

def lockup(pal, bg=None, mono=None):
    h=392; k=h/392; fr=Frame(0, 0, h); s=fr.s
    x0=ADV*s - 10*s   # "ova" follows the N's advance with the wordmark's tracking
    word="".join(f'<path transform="translate({x0+px*(h/OVA["cap"]):.2f},0) scale({h/OVA["cap"]:.4f},{-h/OVA["cap"]:.4f})" d="{d}" fill="{mono or PALETTES[pal]["word"]}"/>' for px,d in OVA["paths"])
    left=45.4*s-36; top=-h-168*k-34; right=x0+OVA["advance"]*(h/OVA["cap"])+40; bottom=40
    vb=(left,top,right-left,bottom-top)
    back=f'<rect x="{vb[0]:.0f}" y="{vb[1]:.0f}" width="{vb[2]:.0f}" height="{vb[3]:.0f}" fill="{bg}"/>' if bg else ""
    glow=f'<circle cx="{fr.star[0]:.1f}" cy="{fr.star[1]:.1f}" r="240" fill="url(#glow)"/>' if (pal=="dark" and not mono) else ""
    d = "" if mono else f"<defs>{defs(fr,pal,k,240 if pal=='dark' else None)}</defs>"
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{" ".join(f"{v:.0f}" for v in vb)}" width="{vb[2]:.0f}" height="{vb[3]:.0f}">'
            f'{d}{back}{glow}{mark(fr,k,mono=mono)}{word}</svg>')

def symbol(pal, mono=None):
    h=392; k=1; fr=Frame(0,0,h); s=fr.s
    left=45.4*s-24; right=max(630.6*s, fr.star[0]+124*k)+24; top=-h-168*k-24; bottom=24
    vb=(left,top,right-left,bottom-top)
    d = "" if mono else f"<defs>{defs(fr,pal,k)}</defs>"
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{" ".join(f"{v:.0f}" for v in vb)}" width="{vb[2]:.0f}" height="{vb[3]:.0f}">{d}{mark(fr,k,mono=mono)}</svg>'

if __name__=="__main__":
    open("icon.svg","w").write(icon())
    open("icon-small.svg","w").write(icon(small=True))
    open("lockup-dark.svg","w").write(lockup("dark"))
    open("lockup-light.svg","w").write(lockup("light"))
    open("prev-lockup-dark.svg","w").write(lockup("dark","#0a0a0c"))
    open("prev-lockup-light.svg","w").write(lockup("light","#f6f4f1"))
    open("symbol-dark.svg","w").write(symbol("dark"))
    open("symbol-light.svg","w").write(symbol("light"))
    open("lockup-mono-white.svg","w").write(lockup("dark",mono="#ffffff"))
    open("lockup-mono-black.svg","w").write(lockup("light",mono="#000000"))
    print("ok")
