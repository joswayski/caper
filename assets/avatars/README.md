# Caper character catalog

This is the human-readable catalog of Caper's 100 account-avatar designs. It is
also a naming proposal for possible future emoji reactions: the slugs below are
**catalog labels, not a shipped reaction API**, and reactions are not implemented
by this document.

## IDs, variants, and provenance

An account stores `avatar_id = hue * 100 + design`, where `design` is 0–99 and
`hue` is 0–7. Thus the 800 persisted IDs are eight color variants of these 100
characters, not 800 different characters. Hue 0 is cataloged below; the other
variants rotate the palette in 45° steps and retain the same design and label.
Current assets are served at `/images/avatars/v3/{avatar_id}.svg`.

IDs are frozen account data. Do not reorder, reuse, or replace them; a future
collection needs a versioned schema and asset path. The original source artwork
is AI-generated companion artwork based on Caper's own mascot and
`apps/web/public/images/demo-avatars.webp`; no Puck artwork is copied or bundled.
Designs 0–3 came from that original demo sheet. Designs 4–99 came, four at a
time in reading order, from `companions-01.webp` through
`companions-24.webp`. Those 512×512 WebP source sheets are retained here for
provenance and are not served. The current v3 vectors include later hand repairs;
in particular, the rendered designs are 15 safari hat, 61 carrot cap, 64 cat,
and 69 dinosaur. There is no sombrero design in the current artwork.

`node scripts/generate-avatars.mjs` reproduces the original immutable v1 raster
atlas from the sheets (ImageMagick 7). Current vectors are exported by
`node scripts/generate-avatar-vectors.mjs`; `--check` verifies all client copies.

Website wordmarks and rotating favicons use `/images/branding/v1/{id}.svg`.
Native in-app wordmarks bundle the same transparent artwork: Rust's `BRANDING`
table, Apple's `caper-branding-{id}` assets and Android's `caper_branding_{id}`
drawables. The generator removes only the background path and circular crop;
character paths, hues and daily selections are unchanged. Profile avatars and
external window/Dock/launcher icons still use the original circular artwork.
Installed website touch/manifest icons and the iOS home-screen icon stay fixed.

## Original mascot (not an account design)

`plain-caper` is the plain Caper dot. It predates the account set and has no design ID. Keep it
separate from the 0–99 catalog: [![Original plain Caper dot](../../apps/web/public/caper-face.svg)](../../apps/web/public/caper-face.svg).

## Designs

Each preview is the current hue-0 v3 asset and links to the full SVG.

| Preview | Design ID | Proposed label | Description |
| --- | ---: | --- | --- |
| [![Beanie Caper](../../apps/web/public/images/avatars/v3/0.svg)](../../apps/web/public/images/avatars/v3/0.svg) | 0 | `beanie-caper` | Purple knit beanie with a pom-pom. |
| [![Headphones Caper](../../apps/web/public/images/avatars/v3/1.svg)](../../apps/web/public/images/avatars/v3/1.svg) | 1 | `headphones-caper` | Large cream over-ear headphones. |
| [![Round Glasses Caper](../../apps/web/public/images/avatars/v3/2.svg)](../../apps/web/public/images/avatars/v3/2.svg) | 2 | `round-glasses-caper` | Round spectacles with offset pupils. |
| [![Sleepy Star Caper](../../apps/web/public/images/avatars/v3/3.svg)](../../apps/web/public/images/avatars/v3/3.svg) | 3 | `sleepy-star-caper` | Sleeping face with a yellow star. |
| [![Backward Cap Caper](../../apps/web/public/images/avatars/v3/4.svg)](../../apps/web/public/images/avatars/v3/4.svg) | 4 | `backward-cap-caper` | Coral baseball cap worn backward. |
| [![Scarf Caper](../../apps/web/public/images/avatars/v3/5.svg)](../../apps/web/public/images/avatars/v3/5.svg) | 5 | `scarf-caper` | Cozy green scarf around the base. |
| [![Astronaut Caper](../../apps/web/public/images/avatars/v3/6.svg)](../../apps/web/public/images/avatars/v3/6.svg) | 6 | `astronaut-caper` | Rounded space helmet with glass visor. |
| [![Daisy Caper](../../apps/web/public/images/avatars/v3/7.svg)](../../apps/web/public/images/avatars/v3/7.svg) | 7 | `daisy-caper` | Small cream daisy tucked at the side. |
| [![Cowboy Caper](../../apps/web/public/images/avatars/v3/8.svg)](../../apps/web/public/images/avatars/v3/8.svg) | 8 | `cowboy-caper` | Brown cowboy hat with a star badge. |
| [![Crown Caper](../../apps/web/public/images/avatars/v3/9.svg)](../../apps/web/public/images/avatars/v3/9.svg) | 9 | `crown-caper` | Gold three-point crown with a red jewel. |
| [![Pirate Caper](../../apps/web/public/images/avatars/v3/10.svg)](../../apps/web/public/images/avatars/v3/10.svg) | 10 | `pirate-caper` | Skull hat, eye patch, and curled smile. |
| [![Red Hood Caper](../../apps/web/public/images/avatars/v3/11.svg)](../../apps/web/public/images/avatars/v3/11.svg) | 11 | `red-hood-caper` | Red tied hood framing a yellow face. |
| [![Chef Caper](../../apps/web/public/images/avatars/v3/12.svg)](../../apps/web/public/images/avatars/v3/12.svg) | 12 | `chef-caper` | Tall white chef's toque. |
| [![Wizard Caper](../../apps/web/public/images/avatars/v3/13.svg)](../../apps/web/public/images/avatars/v3/13.svg) | 13 | `wizard-caper` | Blue pointed hat with a gold star. |
| [![Bandage Caper](../../apps/web/public/images/avatars/v3/14.svg)](../../apps/web/public/images/avatars/v3/14.svg) | 14 | `bandage-caper` | Small adhesive bandage on the forehead. |
| [![Safari Hat Caper](../../apps/web/public/images/avatars/v3/15.svg)](../../apps/web/public/images/avatars/v3/15.svg) | 15 | `safari-hat-caper` | Yellow ridged safari hat with a broad brim. |
| [![Cat Ears Caper](../../apps/web/public/images/avatars/v3/16.svg)](../../apps/web/public/images/avatars/v3/16.svg) | 16 | `cat-ears-caper` | Pink-lined cat ears. |
| [![Bunny Ears Caper](../../apps/web/public/images/avatars/v3/17.svg)](../../apps/web/public/images/avatars/v3/17.svg) | 17 | `bunny-ears-caper` | Tall cream rabbit ears with pink centers. |
| [![Sprout Caper](../../apps/web/public/images/avatars/v3/18.svg)](../../apps/web/public/images/avatars/v3/18.svg) | 18 | `sprout-caper` | Two-leaf green sprout on top. |
| [![Devil Caper](../../apps/web/public/images/avatars/v3/19.svg)](../../apps/web/public/images/avatars/v3/19.svg) | 19 | `devil-caper` | Red horns and a mischievous expression. |
| [![Sunglasses Caper](../../apps/web/public/images/avatars/v3/20.svg)](../../apps/web/public/images/avatars/v3/20.svg) | 20 | `sunglasses-caper` | Black angular sunglasses. |
| [![Halo Caper](../../apps/web/public/images/avatars/v3/21.svg)](../../apps/web/public/images/avatars/v3/21.svg) | 21 | `halo-caper` | Slim gold halo floating overhead. |
| [![Antennae Caper](../../apps/web/public/images/avatars/v3/22.svg)](../../apps/web/public/images/avatars/v3/22.svg) | 22 | `antennae-caper` | Pair of bobble-tipped antennae. |
| [![Beret Caper](../../apps/web/public/images/avatars/v3/23.svg)](../../apps/web/public/images/avatars/v3/23.svg) | 23 | `beret-caper` | Red beret and tiny curled moustache. |
| [![Sweatband Caper](../../apps/web/public/images/avatars/v3/24.svg)](../../apps/web/public/images/avatars/v3/24.svg) | 24 | `sweatband-caper` | Striped athletic headband. |
| [![Earmuffs Caper](../../apps/web/public/images/avatars/v3/25.svg)](../../apps/web/public/images/avatars/v3/25.svg) | 25 | `earmuffs-caper` | Cream winter earmuffs. |
| [![Top Hat Caper](../../apps/web/public/images/avatars/v3/26.svg)](../../apps/web/public/images/avatars/v3/26.svg) | 26 | `top-hat-caper` | Small black top hat with a red band. |
| [![Sleep Mask Caper](../../apps/web/public/images/avatars/v3/27.svg)](../../apps/web/public/images/avatars/v3/27.svg) | 27 | `sleep-mask-caper` | Cream sleep mask over closed eyes. |
| [![Bow Caper](../../apps/web/public/images/avatars/v3/28.svg)](../../apps/web/public/images/avatars/v3/28.svg) | 28 | `bow-caper` | Large cream hair bow. |
| [![Hard Hat Caper](../../apps/web/public/images/avatars/v3/29.svg)](../../apps/web/public/images/avatars/v3/29.svg) | 29 | `hard-hat-caper` | Small yellow construction helmet. |
| [![Butterfly Caper](../../apps/web/public/images/avatars/v3/30.svg)](../../apps/web/public/images/avatars/v3/30.svg) | 30 | `butterfly-caper` | Orange butterfly perched overhead. |
| [![Superhero Caper](../../apps/web/public/images/avatars/v3/31.svg)](../../apps/web/public/images/avatars/v3/31.svg) | 31 | `superhero-caper` | Red tied eye mask. |
| [![Aviator Caper](../../apps/web/public/images/avatars/v3/32.svg)](../../apps/web/public/images/avatars/v3/32.svg) | 32 | `aviator-caper` | Leather flight cap and silver goggles. |
| [![Laurel Caper](../../apps/web/public/images/avatars/v3/33.svg)](../../apps/web/public/images/avatars/v3/33.svg) | 33 | `laurel-caper` | Green laurel leaves sweeping across the brow. |
| [![Party Hat Caper](../../apps/web/public/images/avatars/v3/34.svg)](../../apps/web/public/images/avatars/v3/34.svg) | 34 | `party-hat-caper` | Red-and-white striped party cone. |
| [![Frog Caper](../../apps/web/public/images/avatars/v3/35.svg)](../../apps/web/public/images/avatars/v3/35.svg) | 35 | `frog-caper` | Green frog hood with raised eyes. |
| [![Shark Caper](../../apps/web/public/images/avatars/v3/36.svg)](../../apps/web/public/images/avatars/v3/36.svg) | 36 | `shark-caper` | Blue shark hood with white teeth. |
| [![Strawberry Caper](../../apps/web/public/images/avatars/v3/37.svg)](../../apps/web/public/images/avatars/v3/37.svg) | 37 | `strawberry-caper` | Red seeded strawberry cap. |
| [![Mushroom Caper](../../apps/web/public/images/avatars/v3/38.svg)](../../apps/web/public/images/avatars/v3/38.svg) | 38 | `mushroom-caper` | Red mushroom cap with cream spots. |
| [![Pumpkin Caper](../../apps/web/public/images/avatars/v3/39.svg)](../../apps/web/public/images/avatars/v3/39.svg) | 39 | `pumpkin-caper` | Ridged orange pumpkin body and curling vine. |
| [![Lemon Caper](../../apps/web/public/images/avatars/v3/40.svg)](../../apps/web/public/images/avatars/v3/40.svg) | 40 | `lemon-caper` | Lemon wedge balanced on top. |
| [![Watermelon Caper](../../apps/web/public/images/avatars/v3/41.svg)](../../apps/web/public/images/avatars/v3/41.svg) | 41 | `watermelon-caper` | Watermelon slice worn as a tilted cap. |
| [![Fried Egg Caper](../../apps/web/public/images/avatars/v3/42.svg)](../../apps/web/public/images/avatars/v3/42.svg) | 42 | `fried-egg-caper` | Sunny-side-up egg on top. |
| [![Croissant Caper](../../apps/web/public/images/avatars/v3/43.svg)](../../apps/web/public/images/avatars/v3/43.svg) | 43 | `croissant-caper` | Golden croissant curved over the head. |
| [![Sushi Caper](../../apps/web/public/images/avatars/v3/44.svg)](../../apps/web/public/images/avatars/v3/44.svg) | 44 | `sushi-caper` | Salmon nigiri with a dark seaweed band. |
| [![Ice Cream Caper](../../apps/web/public/images/avatars/v3/45.svg)](../../apps/web/public/images/avatars/v3/45.svg) | 45 | `ice-cream-caper` | Upside-down sprinkled ice-cream cone. |
| [![Popcorn Caper](../../apps/web/public/images/avatars/v3/46.svg)](../../apps/web/public/images/avatars/v3/46.svg) | 46 | `popcorn-caper` | Striped carton overflowing with popcorn. |
| [![Teacup Caper](../../apps/web/public/images/avatars/v3/47.svg)](../../apps/web/public/images/avatars/v3/47.svg) | 47 | `teacup-caper` | White floral teacup balanced on top. |
| [![Viking Caper](../../apps/web/public/images/avatars/v3/48.svg)](../../apps/web/public/images/avatars/v3/48.svg) | 48 | `viking-caper` | Brown riveted helmet with cream horns. |
| [![Knight Caper](../../apps/web/public/images/avatars/v3/49.svg)](../../apps/web/public/images/avatars/v3/49.svg) | 49 | `knight-caper` | Silver barred helmet with a red plume. |
| [![Jester Caper](../../apps/web/public/images/avatars/v3/50.svg)](../../apps/web/public/images/avatars/v3/50.svg) | 50 | `jester-caper` | Two-tone jester cap with gold bells. |
| [![Detective Caper](../../apps/web/public/images/avatars/v3/51.svg)](../../apps/web/public/images/avatars/v3/51.svg) | 51 | `detective-caper` | Brown checked deerstalker hat. |
| [![Sailor Caper](../../apps/web/public/images/avatars/v3/52.svg)](../../apps/web/public/images/avatars/v3/52.svg) | 52 | `sailor-caper` | White sailor cap with a blue anchor. |
| [![Firefighter Caper](../../apps/web/public/images/avatars/v3/53.svg)](../../apps/web/public/images/avatars/v3/53.svg) | 53 | `firefighter-caper` | Red fire helmet with a gold shield. |
| [![Nurse Caper](../../apps/web/public/images/avatars/v3/54.svg)](../../apps/web/public/images/avatars/v3/54.svg) | 54 | `nurse-caper` | White nurse cap with a heart badge. |
| [![Graduate Caper](../../apps/web/public/images/avatars/v3/55.svg)](../../apps/web/public/images/avatars/v3/55.svg) | 55 | `graduate-caper` | Black mortarboard and gold tassel. |
| [![Trapper Hat Caper](../../apps/web/public/images/avatars/v3/56.svg)](../../apps/web/public/images/avatars/v3/56.svg) | 56 | `trapper-hat-caper` | Fur-lined winter trapper hat. |
| [![Sun Hat Caper](../../apps/web/public/images/avatars/v3/57.svg)](../../apps/web/public/images/avatars/v3/57.svg) | 57 | `sun-hat-caper` | Wide yellow sun hat with a daisy. |
| [![Fishing Hat Caper](../../apps/web/public/images/avatars/v3/58.svg)](../../apps/web/public/images/avatars/v3/58.svg) | 58 | `fishing-hat-caper` | Green cap with a hooked lure. |
| [![Marching Band Caper](../../apps/web/public/images/avatars/v3/59.svg)](../../apps/web/public/images/avatars/v3/59.svg) | 59 | `marching-band-caper` | Red shako with a white feather plume. |
| [![Paper Boat Caper](../../apps/web/public/images/avatars/v3/60.svg)](../../apps/web/public/images/avatars/v3/60.svg) | 60 | `paper-boat-caper` | Folded white paper boat hat. |
| [![Carrot Cap Caper](../../apps/web/public/images/avatars/v3/61.svg)](../../apps/web/public/images/avatars/v3/61.svg) | 61 | `carrot-cap-caper` | Orange carrot cap with a green sprout. |
| [![Acorn Caper](../../apps/web/public/images/avatars/v3/62.svg)](../../apps/web/public/images/avatars/v3/62.svg) | 62 | `acorn-caper` | Brown ridged acorn cap. |
| [![Pinecone Caper](../../apps/web/public/images/avatars/v3/63.svg)](../../apps/web/public/images/avatars/v3/63.svg) | 63 | `pinecone-caper` | Layered brown pinecone cap. |
| [![Cat Caper](../../apps/web/public/images/avatars/v3/64.svg)](../../apps/web/public/images/avatars/v3/64.svg) | 64 | `cat-caper` | Orange cat face with cream inner ears. |
| [![Bear Caper](../../apps/web/public/images/avatars/v3/65.svg)](../../apps/web/public/images/avatars/v3/65.svg) | 65 | `bear-caper` | Brown bear face with round ears. |
| [![Panda Caper](../../apps/web/public/images/avatars/v3/66.svg)](../../apps/web/public/images/avatars/v3/66.svg) | 66 | `panda-caper` | Cream panda face with black ears and eye patches. |
| [![Koala Caper](../../apps/web/public/images/avatars/v3/67.svg)](../../apps/web/public/images/avatars/v3/67.svg) | 67 | `koala-caper` | Gray koala face with fluffy ears. |
| [![Tiger Caper](../../apps/web/public/images/avatars/v3/68.svg)](../../apps/web/public/images/avatars/v3/68.svg) | 68 | `tiger-caper` | Orange tiger face with dark stripes. |
| [![Dinosaur Caper](../../apps/web/public/images/avatars/v3/69.svg)](../../apps/web/public/images/avatars/v3/69.svg) | 69 | `dinosaur-caper` | Green horned dinosaur with back plates. |
| [![Unicorn Caper](../../apps/web/public/images/avatars/v3/70.svg)](../../apps/web/public/images/avatars/v3/70.svg) | 70 | `unicorn-caper` | Pink-and-blue mane with a striped horn. |
| [![Sheep Caper](../../apps/web/public/images/avatars/v3/71.svg)](../../apps/web/public/images/avatars/v3/71.svg) | 71 | `sheep-caper` | Fluffy cream wool and small pink ears. |
| [![Bee Caper](../../apps/web/public/images/avatars/v3/72.svg)](../../apps/web/public/images/avatars/v3/72.svg) | 72 | `bee-caper` | Yellow-and-brown bee hood with antennae. |
| [![Ladybug Caper](../../apps/web/public/images/avatars/v3/73.svg)](../../apps/web/public/images/avatars/v3/73.svg) | 73 | `ladybug-caper` | Red spotted ladybug shell and antennae. |
| [![Octopus Caper](../../apps/web/public/images/avatars/v3/74.svg)](../../apps/web/public/images/avatars/v3/74.svg) | 74 | `octopus-caper` | Purple octopus with curled tentacles. |
| [![Crab Caper](../../apps/web/public/images/avatars/v3/75.svg)](../../apps/web/public/images/avatars/v3/75.svg) | 75 | `crab-caper` | Red crab hood with raised claws and eyes. |
| [![Penguin Caper](../../apps/web/public/images/avatars/v3/76.svg)](../../apps/web/public/images/avatars/v3/76.svg) | 76 | `penguin-caper` | Blue penguin hood with a yellow beak. |
| [![Owl Caper](../../apps/web/public/images/avatars/v3/77.svg)](../../apps/web/public/images/avatars/v3/77.svg) | 77 | `owl-caper` | Brown owl hood with large cream eyes. |
| [![Snail Caper](../../apps/web/public/images/avatars/v3/78.svg)](../../apps/web/public/images/avatars/v3/78.svg) | 78 | `snail-caper` | Brown spiral snail shell at the side. |
| [![Turtle Caper](../../apps/web/public/images/avatars/v3/79.svg)](../../apps/web/public/images/avatars/v3/79.svg) | 79 | `turtle-caper` | Green striped turtle shell cap. |
| [![Snorkel Caper](../../apps/web/public/images/avatars/v3/80.svg)](../../apps/web/public/images/avatars/v3/80.svg) | 80 | `snorkel-caper` | Blue dive mask and yellow-tipped snorkel. |
| [![Ski Goggles Caper](../../apps/web/public/images/avatars/v3/81.svg)](../../apps/web/public/images/avatars/v3/81.svg) | 81 | `ski-goggles-caper` | Dark ski goggles and orange knit cap. |
| [![Cyclist Caper](../../apps/web/public/images/avatars/v3/82.svg)](../../apps/web/public/images/avatars/v3/82.svg) | 82 | `cyclist-caper` | Blue vented cycling helmet. |
| [![Racer Caper](../../apps/web/public/images/avatars/v3/83.svg)](../../apps/web/public/images/avatars/v3/83.svg) | 83 | `racer-caper` | White racing helmet with red stripe. |
| [![Jeweled Turban Caper](../../apps/web/public/images/avatars/v3/84.svg)](../../apps/web/public/images/avatars/v3/84.svg) | 84 | `jeweled-turban-caper` | Cream wrapped turban with a red jewel. |
| [![Future Visor Caper](../../apps/web/public/images/avatars/v3/85.svg)](../../apps/web/public/images/avatars/v3/85.svg) | 85 | `future-visor-caper` | Silver glowing futuristic visor. |
| [![Moon Cap Caper](../../apps/web/public/images/avatars/v3/86.svg)](../../apps/web/public/images/avatars/v3/86.svg) | 86 | `moon-cap-caper` | Purple nightcap with a gold crescent. |
| [![Rainbow Cloud Caper](../../apps/web/public/images/avatars/v3/87.svg)](../../apps/web/public/images/avatars/v3/87.svg) | 87 | `rainbow-cloud-caper` | Small cloud and rainbow overhead. |
| [![Snowflake Caper](../../apps/web/public/images/avatars/v3/88.svg)](../../apps/web/public/images/avatars/v3/88.svg) | 88 | `snowflake-caper` | Bright blue snowflake at the brow. |
| [![Lightning Caper](../../apps/web/public/images/avatars/v3/89.svg)](../../apps/web/public/images/avatars/v3/89.svg) | 89 | `lightning-caper` | Yellow lightning bolt at the brow. |
| [![Crystal Caper](../../apps/web/public/images/avatars/v3/90.svg)](../../apps/web/public/images/avatars/v3/90.svg) | 90 | `crystal-caper` | Cluster of blue and violet crystals. |
| [![Volcano Caper](../../apps/web/public/images/avatars/v3/91.svg)](../../apps/web/public/images/avatars/v3/91.svg) | 91 | `volcano-caper` | Small erupting brown volcano. |
| [![Cactus Caper](../../apps/web/public/images/avatars/v3/92.svg)](../../apps/web/public/images/avatars/v3/92.svg) | 92 | `cactus-caper` | Flowering green cactus garden. |
| [![Bonsai Caper](../../apps/web/public/images/avatars/v3/93.svg)](../../apps/web/public/images/avatars/v3/93.svg) | 93 | `bonsai-caper` | Tiny leafy bonsai in a brown pot. |
| [![Umbrella Caper](../../apps/web/public/images/avatars/v3/94.svg)](../../apps/web/public/images/avatars/v3/94.svg) | 94 | `umbrella-caper` | Colorful umbrella opened overhead. |
| [![Propeller Caper](../../apps/web/public/images/avatars/v3/95.svg)](../../apps/web/public/images/avatars/v3/95.svg) | 95 | `propeller-caper` | Red-and-white propeller beanie. |
| [![Robot Caper](../../apps/web/public/images/avatars/v3/96.svg)](../../apps/web/public/images/avatars/v3/96.svg) | 96 | `robot-caper` | Silver robot helmet with pink antennae. |
| [![Maple Leaves Caper](../../apps/web/public/images/avatars/v3/97.svg)](../../apps/web/public/images/avatars/v3/97.svg) | 97 | `maple-leaves-caper` | Crown of orange autumn maple leaves. |
| [![Music Notes Caper](../../apps/web/public/images/avatars/v3/98.svg)](../../apps/web/public/images/avatars/v3/98.svg) | 98 | `music-notes-caper` | Pink and cream music notes overhead. |
| [![Sun Cloud Caper](../../apps/web/public/images/avatars/v3/99.svg)](../../apps/web/public/images/avatars/v3/99.svg) | 99 | `sun-cloud-caper` | Puffy blue cloud with a peeking sun. |
