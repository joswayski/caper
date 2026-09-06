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
