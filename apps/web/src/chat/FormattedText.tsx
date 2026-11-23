import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import { Check, Copy } from "lucide-react";
import type { FormattedRun } from "./formatting.ts";
import { highlightCode, languageDisplayNames, type MessageBlock } from "./code.ts";

/** Hidden until activated; its links and mentions cannot be reached before then. */
function Spoiler({ children }: { children: ReactNode }) {
  const [revealed, setRevealed] = useState(false);
  if (revealed)
    return (
      <span className="chat-text-spoiler" data-revealed="">
        {children}
      </span>
    );
  const reveal = (event: { preventDefault(): void; stopPropagation(): void }) => {
    event.preventDefault();
    event.stopPropagation();
    setRevealed(true);
  };
  return (
    <span
      className="chat-text-spoiler"
      role="button"
      tabIndex={0}
      aria-label="Spoiler, reveal"
      onClick={reveal}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") reveal(event);
      }}
    >
      <span inert>{children}</span>
    </span>
  );
}

/** Displayed-text ranges of quoted and unquoted runs, in order. */
export function quoteBlocks(runs: FormattedRun[]) {
  const blocks: { start: number; end: number; quote: boolean }[] = [];
  let offset = 0;
  for (const run of runs) {
    const last = blocks.at(-1);
    if (last && last.quote === run.quote) last.end += run.text.length;
    else blocks.push({ start: offset, end: offset + run.text.length, quote: run.quote });
    offset += run.text.length;
  }
  return blocks;
}

/** A quoted block's bar; newlines around it stay in the text. */
export function QuoteBlock({ quote, children }: { quote: boolean; children: ReactNode }) {
  return quote ? <span className="chat-quote">{children}</span> : <>{children}</>;
}

/** Fenced code block card with language badge, copy button, and syntax highlighting. */
export function CodeBlockView({ block }: { block: Extract<MessageBlock, { kind: "code" }> }) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);

  const tokens = highlightCode(block.code, block.language);
  const label = languageDisplayNames[block.language];

  const copy = (event: { preventDefault(): void; stopPropagation(): void }) => {
    event.preventDefault();
    event.stopPropagation();
    void navigator.clipboard?.writeText(block.code).then(() => {
      setCopied(true);
      clearTimeout(timer.current);
      timer.current = setTimeout(() => setCopied(false), 1800);
    });
  };

  return (
    <div className="chat-code-block" data-language={block.language}>
      <div className="chat-code-header">
        <span className="chat-code-lang">{label}</span>
        <button
          type="button"
          className="chat-code-copy"
          aria-label={copied ? "Copied code" : "Copy code"}
          onClick={copy}
        >
          {copied ? <Check size={13} aria-hidden="true" /> : <Copy size={13} aria-hidden="true" />}
          <span>{copied ? "Copied" : "Copy"}</span>
        </button>
      </div>
      <pre className="chat-code-pre">
        <code>
          {tokens.map((span) => {
            const slice = block.code.slice(span.start, span.end);
            return span.kind === "plain" ? (
              <Fragment key={span.start}>{slice}</Fragment>
            ) : (
              <span key={span.start} className={`tok-${span.kind}`}>
                {slice}
              </span>
            );
          })}
        </code>
      </pre>
    </div>
  );
}

/** Only the message subset is interpreted; HTML and other Markdown remain text. Quotes are blocked by callers. */
export default function FormattedText({
  runs,
  range = { start: 0, end: Infinity },
  children = (text) => text,
}: {
  runs: FormattedRun[];
  range?: { start: number; end: number };
  children?: (text: string) => ReactNode;
}) {
  const pieces: { spoiler: boolean; content: ReactNode }[] = [];
  let offset = 0;
  runs.forEach((run, index) => {
    const start = offset;
    offset += run.text.length;
    const text = run.text.slice(Math.max(0, range.start - start), Math.max(0, range.end - start));
    if (!text) return;
    let content: ReactNode = run.code ? <code className="chat-inline-code">{text}</code> : children(text);
    if (run.strikethrough) content = <s>{content}</s>;
    if (run.underline) content = <u>{content}</u>;
    if (run.italic) content = <em className="chat-italic">{content}</em>;
    if (run.bold) content = <strong>{content}</strong>;
    pieces.push({ spoiler: run.spoiler, content: <Fragment key={index}>{content}</Fragment> });
  });
  // Neighbouring spoiler runs, such as one with a bold word, reveal together.
  const out: ReactNode[] = [];
  for (let index = 0; index < pieces.length;) {
    if (!pieces[index].spoiler) {
      out.push(pieces[index++].content);
      continue;
    }
    const group: ReactNode[] = [];
    const key = index;
    while (index < pieces.length && pieces[index].spoiler) group.push(pieces[index++].content);
    out.push(<Spoiler key={`spoiler-${key}`}>{group}</Spoiler>);
  }
  return out;
}
