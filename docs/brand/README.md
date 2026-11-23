# Caper visual references

These owner-supplied references describe the intended brand, not implemented product features.

- [Identity and style guide](identity.png): wordmark, character, colors, type (shown in Satoshi; the apps now use Nunito and Figtree), and controls.
- [Conversation layout](conversation.png): neutral chrome, channel selection, conversation and thread hierarchy.
- [Call layout](call.png): screen-sharing stage, participant strip, and restrained call controls.

Canonical theme tokens live in [`shared/themes/obsidian.json`](../../shared/themes/obsidian.json) (schema [`shared/themes/schema.json`](../../shared/themes/schema.json), synced across all clients by `node scripts/generate-themes.mjs`): deep obsidian frame `#090A0C`, sidebar `#0D0E11`, inset conversation `#121317`, raised surface `#191B22`, translucent hairline borders (`white @ 8%` / solid `#24262F`), crisp chalk text `#F4F4F6`, chalk primary action `#F4F4F6` with `#090A0C` ink labels, restrained botanical Caper green `#638E46` (mascot) / `#4E7835` & `#77A659` (focus, mentions, ready send), and calm emerald `#4CC970` (live voice/presence). Use concentric corner radii (`18px` canvas/dialogs, `16px` cards, `20px` capsule composer, `10px` controls/rows, `999px` pills). Motion is short and decelerating (`--motion-*`, `--ease-out` in `shared/design.css`); nothing teleports, and reduced-motion settings turn it off. The examples above are historical visual references, not image assets to slice into a working UI.

Thread unread indicators use the existing gold `#E4C76A` (`--pin-gold`) for both the dot and label, distinct from the accent reply count. This does not change channel or DM unread colors.

Source: images supplied by Jose Valerio.

## Echo mark

Caper's app mark across Web, Apple (macOS and iOS), Android, Windows, and Linux. Two concentric c's (caper, chat) read as an echo, a voice carrying outward, around the Caper character. [Overview sheet](echo/echo-sheet.png).

- [`echo.svg`](echo/echo.svg): primary, on dark backgrounds.
- [`echo-plain.svg`](echo/echo-plain.svg): the caper without its face, where the mark should be quieter.
- [`echo-light-background.svg`](echo/echo-light-background.svg): on light backgrounds.
- [`echo-one-color-light.svg`](echo/echo-one-color-light.svg) and [`echo-one-color-dark.svg`](echo/echo-one-color-dark.svg): single-color uses, such as notification icons.
- [`echo-small.svg`](echo/echo-small.svg): below 24px, with heavier rings and the caper without face or shine.
- [`echo-app-icon.svg`](echo/echo-app-icon.svg): 1024px square on obsidian `#0B0B0C`; each platform applies its own corner mask.
- [`echo-lockup-horizontal.svg`](echo/echo-lockup-horizontal.svg) and [`echo-lockup-stacked.svg`](echo/echo-lockup-stacked.svg): with the existing letterforms from `apps/web/public/caper-wordmark-letters.svg`.

Construction, on a 100-unit viewBox centered at (50, 50): rings of radius 40 and 23, stroke 10 with round caps, 7 between the rings, and an opening 42° above and below horizontal. The caper is the unchanged character path from `apps/web/public/caper-wordmark.svg` (the same path as `caper-face.svg`), 26 units wide and centered at (50.3, 50.4); its eyes, smile and shine are even-odd cut-outs, so they show the background. The small cut uses radii 39.5 and 20.5, stroke 13, a 44° opening and a 22-unit caper. Rings are warm chalk `#F4F3EF` (obsidian on light backgrounds); botanical green `#6F9948` stays on the caper.

Proposed motion, not built: _speaking_ pulses the inner ring and then the outer ring, from 38% to full opacity and 97% to 102% scale over 1.4s; _connecting_ turns the inner ring every 1.6s and the outer ring the other way every 2.4s; _quiet_ holds both rings at 28% opacity. Respect reduced-motion settings.

`scripts/generate-favicons.mjs` renders the web, desktop, macOS, iOS, and Android app icons from `echo-app-icon.svg` and keeps `apps/web/public/caper-echo.svg` in sync with `echo.svg`. Style reference: the reductive marks in Yasaburo Kuwayama's _Trade Marks & Symbols, Volume 2: Symbolical Designs_. The mark itself is original and copies none of the book's trademarks.
