# Caper visual references

These owner-supplied references describe the intended brand, not implemented product features.

- [Identity and style guide](identity.png): wordmark, character, colors, Satoshi type, and controls.
- [Conversation layout](conversation.png): neutral chrome, channel selection, conversation and thread hierarchy.
- [Call layout](call.png): screen-sharing stage, participant strip, and restrained call controls.

Canonical colors: blackout `#0C0D0F`, surface `#151719`, border `#34383B`, text `#F3F4F5`, terracotta `#B64D32`, caper green `#637A43`. Use `shared/design.css` tokens. Prefer 8px control corners, subtle borders, and Satoshi regular/medium/bold. Green for the character; terracotta for actions. The examples are design references, not image assets to slice into a working UI.

Source: images supplied by Jose Valerio in the browser-media implementation thread, September 6, 2026.

## Echo mark (saved for later, not in the product)

A candidate mark for caper.chat, saved on October 11, 2026 for possible future use. No client uses it yet; the web, Android, Apple and desktop apps still use the wordmark and character icons. Two concentric c's (caper, chat) read as an echo, a voice carrying outward, around the Caper character. [Overview sheet](echo/echo-sheet.png).

- [`echo.svg`](echo/echo.svg): primary, on dark backgrounds.
- [`echo-plain.svg`](echo/echo-plain.svg): the caper without its face, where the mark should be quieter.
- [`echo-light-background.svg`](echo/echo-light-background.svg): on light backgrounds.
- [`echo-one-color-light.svg`](echo/echo-one-color-light.svg) and [`echo-one-color-dark.svg`](echo/echo-one-color-dark.svg): single-color uses, such as notification icons.
- [`echo-small.svg`](echo/echo-small.svg): below 24px, with heavier rings and the caper without face or shine.
- [`echo-app-icon.svg`](echo/echo-app-icon.svg): 1024px square on blackout; each platform applies its own corner mask.
- [`echo-lockup-horizontal.svg`](echo/echo-lockup-horizontal.svg) and [`echo-lockup-stacked.svg`](echo/echo-lockup-stacked.svg): with the existing letterforms from `apps/web/public/caper-wordmark-letters.svg`.

Construction, on a 100-unit viewBox centered at (50, 50): rings of radius 40 and 23, stroke 10 with round caps, 7 between the rings, and an opening 42° above and below horizontal. The caper is the unchanged character path from `apps/web/public/caper-wordmark.svg` (the same path as `caper-face.svg`), 26 units wide and centered at (50.3, 50.4); its eyes, smile and shine are even-odd cut-outs, so they show the background. The small cut uses radii 39.5 and 20.5, stroke 13, a 44° opening and a 22-unit caper. Rings are text `#F3F4F5` (blackout on light backgrounds); green stays on the caper.

Proposed motion, not built: _speaking_ pulses the inner ring and then the outer ring, from 38% to full opacity and 97% to 102% scale over 1.4s; _connecting_ turns the inner ring every 1.6s and the outer ring the other way every 2.4s; _quiet_ holds both rings at 28% opacity. Respect reduced-motion settings.

Adopting it means replacing the favicon and web app icons, Apple (iOS and macOS), Android and desktop icons together, per the cross-platform rule in `AGENTS.md`. `scripts/generate-favicons.mjs` currently renders the web, desktop and macOS icons from `caper-face.svg`; start there with `echo-app-icon.svg`. Style reference: the reductive marks in Yasaburo Kuwayama's _Trade Marks & Symbols, Volume 2: Symbolical Designs_. The mark itself is original and copies none of the book's trademarks.
