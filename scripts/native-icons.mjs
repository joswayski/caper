// Commit the same Lucide vectors used by the web client. No JS or SVG network
// dependency is needed at native runtime. Run after changing web icon versions.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import * as icons from "lucide-react";

const names = {
  archive: "Archive",
  "archive-restore": "ArchiveRestore",
  "arrow-left": "ArrowLeft",
  "arrow-right": "ArrowRight",
  "audio-lines": "AudioLines",
  ban: "Ban",
  bell: "Bell",
  "bell-minus": "BellMinus",
  "bell-off": "BellOff",
  "bell-ring": "BellRing",
  check: "Check",
  "chevron-down": "ChevronDown",
  "chevron-left": "ChevronLeft",
  "chevron-right": "ChevronRight",
  "chevrons-down-up": "ChevronsDownUp",
  "chevrons-up-down": "ChevronsUpDown",
  copy: "Copy",
  "corner-up-right": "CornerUpRight",
  download: "Download",
  ellipsis: "Ellipsis",
  "external-link": "ExternalLink",
  "file-text": "FileText",
  "file-warning": "FileWarning",
  forward: "Forward",
  hash: "Hash",
  "headphone-off": "HeadphoneOff",
  headphones: "Headphones",
  history: "History",
  inbox: "Inbox",
  link: "Link",
  // Private channels use the keyhole lock on every client; the file keeps its name.
  lock: "LockKeyhole",
  "log-out": "LogOut",
  menu: "Menu",
  "message-circle": "MessageCircle",
  "message-square": "MessageSquare",
  mic: "Mic",
  "mic-off": "MicOff",
  paperclip: "Paperclip",
  pause: "Pause",
  pencil: "Pencil",
  "phone-off": "PhoneOff",
  pin: "Pin",
  "pin-off": "PinOff",
  play: "Play",
  plus: "Plus",
  reply: "Reply",
  search: "Search",
  send: "Send",
  settings: "Settings",
  smile: "Smile",
  "smile-plus": "SmilePlus",
  speech: "Speech",
  users: "Users",
  "volume-2": "Volume2",
  "volume-x": "VolumeX",
  x: "X",
};
const output = resolve("apps/native/desktop/resources/icons");
const apple = resolve("apps/native/apple/Resources/CaperIcons.xcassets");
const info = { author: "xcode", version: 1 };
const json = (value) => `${JSON.stringify(value, null, 2)}\n`;
const files = [[`${apple}/Contents.json`, json({ info })]];
for (const [file, name] of Object.entries(names)) {
  const svg = `${renderToStaticMarkup(createElement(icons[name], { color: "#ffffff", size: 24 }))}\n`;
  const image = `${apple}/caper-${file}.imageset`;
  files.push(
    [`${output}/${file}.svg`, svg],
    [`${image}/${file}.svg`, svg],
    [
      `${image}/Contents.json`,
      json({
        images: [{ filename: `${file}.svg`, idiom: "universal" }],
        info,
        properties: { "preserves-vector-representation": true, "template-rendering-intent": "template" },
      }),
    ],
  );
}
// Custom update mascot layers are original artwork, not Lucide icons. Rust
// includes the canonical SVGs directly; Apple needs asset-catalog exports.
for (const part of ["body", "wave", "feet"]) {
  const svg = readFileSync(`assets/update-caper/${part}.svg`, "utf8");
  const image = resolve(`apps/native/apple/Sources/CaperCore/CaperAvatars.xcassets/caper-update-${part}.imageset`);
  files.push(
    [`${image}/${part}.svg`, svg],
    [
      `${image}/Contents.json`,
      json({
        images: [{ filename: `${part}.svg`, idiom: "universal" }],
        info,
        properties: { "preserves-vector-representation": true, "template-rendering-intent": "original" },
      }),
    ],
  );
}
const license = readFileSync("node_modules/lucide-react/LICENSE", "utf8");
files.push([`${output}/LICENSE`, license], [resolve("apps/native/apple/Resources/Lucide-LICENSE.txt"), license]);
for (const [file, content] of files) {
  if (process.argv.includes("--check")) {
    if (readFileSync(file, "utf8") !== content) throw Error(`Native icon drift: ${file}`);
  } else {
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, content);
  }
}
console.log(
  `${process.argv.includes("--check") ? "Verified" : "Exported"} ${Object.keys(names).length} web Lucide icons for Rust and Apple native clients.`,
);
