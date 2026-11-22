# Inrond 40

Every Caper client uses Inrond 40: [Inrond](https://github.com/Hairline-UI/inrond)
1.0, a rounded Inter with a roundness axis, frozen at `ROND 40`. It keeps
Inter's widths, kerning, features, Cyrillic/Greek/Vietnamese coverage and drawn
italic. It is licensed under the SIL Open Font License 1.1 ([OFL.txt](OFL.txt)),
which allows committing, modifying and bundling it; ship `OFL.txt` with any
application that embeds the fonts.

| File | Used by |
| --- | --- |
| `Inrond40-Regular.ttf`, `-Medium`, `-Bold`, `-Black` | Native clients: static weights 400/500/700/900 at the text optical size (14). Families are named `Inrond 40` (Medium and Black also carry the legacy families `Inrond 40 Medium`/`Inrond 40 Black`); PostScript names match the file names. |
| `apps/web/public/fonts/inrond-40/` | Web: woff2 unicode-range subsets, roman and italic, still variable in weight (100-900) and optical size. |
| `apps/web/src/fonts.css` | Web: the generated `@font-face` rules for those subsets. |

All of these are generated from the SHA-256-pinned upstream release by:

```sh
uv run --locked --group fonts python scripts/build_fonts.py
```

Rerun it only to change the release or the roundness (`ROUNDNESS` in the
script), and commit every output together.
