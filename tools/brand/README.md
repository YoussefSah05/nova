# Logo generator

Builds every version of the Nova logo in [`docs/assets/brand/`](../../docs/assets/brand/). The
story and usage rules are in [`docs/brand.md`](../../docs/brand.md).

## How it is built

- The **N** is Bricolage Grotesque's own N at weight 620 (the coordinates are in
  `nova_logo.py`), split into three planes: dim left stroke, beam, bright right stroke.
- The **star** is eight concave rays (four long, four short) around a radial white-hot core,
  centred on the axis of the right stroke.
- The wordmark's **"ova"** outlines are extracted from the font into `ova.json`, so the SVGs
  need no installed fonts.

## Regenerating

```bash
cd tools/brand

# Only when changing weight, tracking or letters: re-extract outlines
# (needs: pip install fonttools brotli)
python extract_glyphs.py \
  ../../node_modules/.pnpm/@fontsource-variable+bricolage-grotesque@5.2.10/node_modules/@fontsource-variable/bricolage-grotesque/files/bricolage-grotesque-latin-standard-normal.woff2 \
  ova 620 -10 > ova.json

# Write the SVGs, then copy them to docs/assets/brand/ with the nova- prefix
python nova_logo.py

# PNG exports (librsvg)
rsvg-convert -w 1024 icon.svg -o nova-icon-1024.png
```
