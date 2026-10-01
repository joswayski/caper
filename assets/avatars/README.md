# Caper avatar artwork

AI-generated companion artwork based on Caper's own mascot and the original
`apps/web/public/images/demo-avatars.webp`. No Puck artwork is copied or bundled.
Source sheets are 512×512 WebP, with four equal quadrants in reading order.

| Design IDs | Source | Designs in reading order |
| --- | --- | --- |
| 0–3 | Original demo sheet | Beanie, headphones, round glasses, sleepy star |
| 4–7 | `companions-01.webp` | Baseball cap, scarf, astronaut, daisy |
| 8–11 | `companions-02.webp` | Cowboy, crown, pirate, red hood |
| 12–15 | `companions-03.webp` | Chef, wizard, bandage, rain hat |
| 16–19 | `companions-04.webp` | Cat ears, bunny ears, sprout, devil |
| 20–23 | `companions-05.webp` | Sunglasses, halo, antennae, beret |
| 24–27 | `companions-06.webp` | Sweatband, earmuffs, top hat, sleep mask |
| 28–31 | `companions-07.webp` | Bow, hardhat, butterfly, superhero |
| 32–35 | `companions-08.webp` | Aviator, laurel, party cone, frog |
| 36–39 | `companions-09.webp` | Shark, strawberry, mushroom, pumpkin |
| 40–43 | `companions-10.webp` | Lemon, watermelon, fried egg, croissant |
| 44–47 | `companions-11.webp` | Sushi, ice cream, popcorn, teacup |
| 48–51 | `companions-12.webp` | Viking, knight, jester, detective |
| 52–55 | `companions-13.webp` | Sailor, firefighter, nurse, graduate |
| 56–59 | `companions-14.webp` | Trapper, sunhat, fishing hat, marching band |
| 60–63 | `companions-15.webp` | Paper boat, flowerpot, acorn, pinecone |
| 64–67 | `companions-16.webp` | Fox, bear, panda, koala |
| 68–71 | `companions-17.webp` | Tiger, dragon, unicorn, sheep |
| 72–75 | `companions-18.webp` | Bee, ladybug, octopus, crab |
| 76–79 | `companions-19.webp` | Penguin, owl, snail, turtle |
| 80–83 | `companions-20.webp` | Snorkel, ski goggles, cyclist, racer |
| 84–87 | `companions-21.webp` | Jeweled turban, future visor, moon cap, rainbow |
| 88–91 | `companions-22.webp` | Snowflake, lightning, crystal, volcano |
| 92–95 | `companions-23.webp` | Cactus, bonsai, umbrella, propeller |
| 96–99 | `companions-24.webp` | Robot, maple leaves, music notes, suncloud |

`node scripts/generate-avatars.mjs` uses ImageMagick 7 to crop the sheets, resize
to 64px, apply eight 45° hue rotations, and mask circular corners. The resulting
800 tiles are packed row-major into the immutable 32×25 `capers-v1` PNG/WebP atlas
under `apps/web/public/images/avatars`. Saved ID = colorway × 100 + design ID.
Only the generated atlas ships to clients; these source sheets are not served.

Once deployed, do not reorder or replace v1 tiles: users' saved profile assignments
refer to these IDs. Future collections need a versioned schema/asset change.
