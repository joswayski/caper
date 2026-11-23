# Caper Fonts

Caper bundles two SIL OFL 1.1 open-source typefaces identically across Web, Apple (macOS/iOS), Android, Windows, and Linux:

- **Nunito** (Bold 700, ExtraBold 800, plus web Black 900): friendly rounded display headings, space/channel titles, welcome headers, and dialogs.
- **Figtree** (Regular 400, Medium 500, Bold 700, Black 900, plus web Italic): clean, friendly UI controls, navigation, and chat messages.

Rebuild both the web WOFF2 subsets (`apps/web/public/fonts/caper/`, `apps/web/src/fonts.css`) and native static TTFs (`shared/fonts/`) from the SHA-256-pinned upstream `google/fonts` commit:

```bash
uv run --locked --group fonts python scripts/build_fonts.py
```

The combined SIL Open Font License 1.1 notice for both families lives in [`OFL.txt`](OFL.txt).
