# Homepage art references

These four PNGs were supplied by Jose as visual references for the homepage's
3D chat illustration. They are design references, not shipped browser assets.

- `homepage-before.png`: original abstract floating chat window.
- `chat-application.png`: target Studio workspace, conversation, and thread layout.
- `caper-pebble.png`: caper bud character and expressions.
- `caper-materials.png`: terracotta, glass, plush, and chrome material explorations.

The new generated avatar sheet lives at
`apps/web/public/images/caper-avatars.webp` (1024 × 1024, four equal quadrants).
Clockwise from the top left: olive Maya, terracotta Jose with glasses, chrome
Sam with a teal leaf, and lavender Alex with a yellow beanie. Created with Amp's
image generator using the supplied character references; no real portraits or
Amp Puck artwork are used.

`apps/web/src/components/chatModel.ts` builds the app from separate Three.js
meshes and canvas-text labels. The avatar sheet is the only downloaded texture;
the game attachment is a procedural low-poly diorama, reused in the thread.
This is an illustrative scene, not an interactive chat client.

WebGL cannot ship in the first HTML response, so rest-pose posters live at
`apps/web/public/images/chat-preview-wide.webp` and
`chat-preview-stacked.webp`. They are the first paint; the live canvas replaces
them after the avatar atlas is ready.

Any change to the hero container size or the camera framing in
`mountChatPreview.ts` must re-sync these files, or first paint will visibly
pop when the live canvas takes over. Recapture the live canvas at rest pose
(reduced motion, 2x DPR): wide viewport for `chat-preview-wide.webp`,
a <=1000px viewport for `chat-preview-stacked.webp`. Keep each file's aspect
equal to its container's, and update the `width`/`height` on the poster `<img>`
in `LowPolyChat.tsx` to match the wide file.

The hero preview keeps `aspect-ratio: 1.42` at every breakpoint, including
phones, so `object-fit: fill` is 1:1 with the live camera. Do not give the
phone container a shorter ratio unless you also recapture a matching poster.
