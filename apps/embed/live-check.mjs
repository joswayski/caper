// Runs every site reader against real public posts, so a site changing its
// pages shows up as a failed check instead of silently missing previews.
//
//   node apps/embed/live-check.mjs                  readers, from this machine
//   EMBED_ORIGIN=https://embed.caper.chat EMBED_API_SECRET=… node apps/embed/live-check.mjs
//                                                   the deployed Worker, from Cloudflare
//
// Instagram is reported but never fails the run: it blocks logged-out
// fetches often enough that a miss is expected.

import { unfurl } from "./sites.mjs";

const CASES = [
  { url: "https://x.com/jack/status/20", expect: ["author", "text", "footer"] },
  { url: "https://x.com/DivineDropbear/status/1841206275088290279", expect: ["author", "video"] },
  { url: "https://x.com/SpaceX/status/1848831595014459513", expect: ["author", "images"] },
  { url: "https://www.tiktok.com/@scout2015/video/6718335390845095173", expect: ["author", "video"] },
  { url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ", expect: ["title", "player"] },
  { url: "https://youtu.be/jNQXAC9IVRw", expect: ["title", "player"] },
  { url: "https://vimeo.com/1084537", expect: ["title", "player"] },
  { url: "https://www.twitch.tv/twitch", expect: ["player"] },
  { url: "https://www.theverge.com/", expect: ["title"] },
  { url: "https://en.wikipedia.org/wiki/Capers", expect: ["title"] },
  { url: "https://www.instagram.com/p/BsOGulcndj-/", expect: ["title"], optional: true },
];

async function viaWorker(url) {
  const response = await fetch(`${process.env.EMBED_ORIGIN}/v1/unfurl`, {
    method: "POST",
    headers: { authorization: `Bearer ${process.env.EMBED_API_SECRET}`, "content-type": "application/json" },
    body: JSON.stringify({ url }),
  });
  if (!response.ok) throw new Error(`unfurl answered ${response.status}`);
  return (await response.json()).embed;
}

let failed = 0;
for (const { url, expect, optional } of CASES) {
  let embed = null;
  let error = null;
  try {
    embed = process.env.EMBED_ORIGIN ? await viaWorker(url) : await unfurl(new URL(url));
  } catch (caught) {
    error = caught;
  }
  const types = embed?.blocks.map((block) => block.type) ?? [];
  const missing = expect.filter((type) => !types.includes(type));
  const ok = embed && !missing.length;
  if (!ok && !optional) failed++;
  const status = ok ? "ok  " : optional ? "skip" : "FAIL";
  console.log(
    `${status} ${url}  [${types.join(", ")}]${missing.length ? ` missing ${missing.join(", ")}` : ""}${error ? ` ${error.message}` : ""}`,
  );
}
process.exitCode = failed ? 1 : 0;
