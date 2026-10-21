import { Fragment, type ReactNode } from "react";
import type { FormattedRun } from "./formatting.ts";

/** Only emphasis is interpreted; HTML and other Markdown remain text. */
export default function FormattedText({
  runs,
  range = { start: 0, end: Infinity },
  children = (text) => text,
}: {
  runs: FormattedRun[];
  range?: { start: number; end: number };
  children?: (text: string) => ReactNode;
}) {
  let offset = 0;
  return runs.map((run, index) => {
    const start = offset;
    offset += run.text.length;
    const text = run.text.slice(Math.max(0, range.start - start), Math.max(0, range.end - start));
    if (!text) return null;
    let content = children(text);
    if (run.italic) content = <em className="chat-italic">{content}</em>;
    if (run.bold) content = <strong>{content}</strong>;
    return <Fragment key={index}>{content}</Fragment>;
  });
}
