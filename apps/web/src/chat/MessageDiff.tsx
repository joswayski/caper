import { useMemo } from "react";
import { preloadHighlighter } from "@pierre/diffs";
import { MultiFileDiff } from "@pierre/diffs/react";
import type { MessageVersion } from "./types.ts";

// Prepare before mounting: StrictMode can otherwise hydrate an empty first-pass pre.
export const ready = preloadHighlighter({ themes: ["pierre-dark"], langs: ["text"] });

const options = {
  theme: "pierre-dark", themeType: "dark", diffStyle: "split", overflow: "wrap",
  expandUnchanged: true, disableFileHeader: true, disableLineNumbers: true,
  lineDiffType: "word", maxLineDiffLength: 8_000, diffIndicators: "classic",
  unsafeCSS: "[data-no-newline], [data-gutter-buffer='metadata'] { display: none; }",
} as const;

export default function MessageDiff({ messageId, before, after }: { messageId: string; before: MessageVersion; after: MessageVersion }) {
  const files = useMemo(() => ({
    oldFile: { name: "message.txt", contents: before.content.text, lang: "text" as const, cacheKey: `${messageId}:${before.revision}` },
    newFile: { name: "message.txt", contents: after.content.text, lang: "text" as const, cacheKey: `${messageId}:${after.revision}` },
  }), [messageId, before, after]);
  return <div className="chat-code-diff">
    <MultiFileDiff {...files} options={options} />
    <div className="sr-only"><p>Before: {before.content.text}</p><p>After: {after.content.text}</p></div>
  </div>;
}
