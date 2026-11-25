import { createLucideIcon } from "lucide-react";

export { CornerUpLeft as Reply } from "lucide-react";

// Lucide Spool (0.519.0, ISC), newer than our installed icon package.
// https://github.com/lucide-icons/lucide/blob/0.519.0/icons/spool.svg
export const Thread = createLucideIcon("Thread", [
  [
    "path",
    {
      d: "M17 13.44 4.442 17.082A2 2 0 0 0 4.982 21H19a2 2 0 0 0 .558-3.921l-1.115-.32A2 2 0 0 1 17 14.837V7.66",
      key: "13vns8",
    },
  ],
  [
    "path",
    {
      d: "m7 10.56 12.558-3.642A2 2 0 0 0 19.018 3H5a2 2 0 0 0-.558 3.921l1.115.32A2 2 0 0 1 7 9.163v7.178",
      key: "s8x3u0",
    },
  ],
]);

// The composer's GIF button: a filled badge with the letters cut out, on
// Lucide's 24-unit grid so it shares the emoji and microphone icons' box.
export const Gif = createLucideIcon("Gif", [
  [
    "path",
    {
      d: "M6 3h12a4 4 0 0 1 4 4v10a4 4 0 0 1-4 4H6a4 4 0 0 1-4-4V7a4 4 0 0 1 4-4ZM4.5 8v8h6v-4.5h-3v1.75h1V14h-2v-4h4V8ZM12 8v8h2V8ZM15.5 8v8h2v-2.75H19v-2h-1.5V10h2V8Z",
      fill: "currentColor",
      fillRule: "evenodd",
      stroke: "none",
      key: "gif",
    },
  ],
]);
