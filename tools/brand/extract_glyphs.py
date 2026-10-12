import sys, json
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.boundsPen import BoundsPen
src, text, wght, track = sys.argv[1], sys.argv[2], float(sys.argv[3]), float(sys.argv[4])
f = instantiateVariableFont(TTFont(src), {"wght": wght, "opsz": 96, "wdth": 100})
cmap = f.getBestCmap(); gs = f.getGlyphSet(); hmtx = f["hmtx"]
# pair kerning from GPOS is skipped; tracking is tuned by eye
x = 0; paths = []
for ch in text:
    g = cmap[ord(ch)]
    pen = SVGPathPen(gs); gs[g].draw(pen)
    paths.append((x, pen.getCommands()))
    x += hmtx[g][0] + track
cap = f["OS/2"].sCapHeight
bp = BoundsPen(gs); 
for i,ch in enumerate(text): pass
print(json.dumps({"paths": paths, "advance": x - track, "cap": cap, "upm": f["head"].unitsPerEm}))
