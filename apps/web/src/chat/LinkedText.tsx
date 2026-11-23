import { Fragment } from "react";
import { Link } from "lucide-react";
import { linkSegments } from "./links.ts";
import FormattedText, { CodeBlockView, QuoteBlock, quoteBlocks } from "./FormattedText.tsx";
import { formattedRuns } from "./formatting.ts";
import { parseMessageBlocks } from "./code.ts";

/** Message formatting and safe autolinks; mention-aware callers format their outer runs. */
export default function LinkedText({ text, formatted = true }: { text: string; formatted?: boolean }) {
  if (formatted) {
    return parseMessageBlocks(text).map((block) => {
      if (block.kind === "code") return <CodeBlockView key={block.start} block={block} />;
      const runs = formattedRuns(block.text);
      return (
        <Fragment key={block.start}>
          {quoteBlocks(runs).map((qblock) => (
            <QuoteBlock key={qblock.start} quote={qblock.quote}>
              <FormattedText runs={runs} range={qblock}>
                {(part) => <LinkedText text={part} formatted={false} />}
              </FormattedText>
            </QuoteBlock>
          ))}
        </Fragment>
      );
    });
  }
  return (
    <>
      {linkSegments(text).map((segment, index) => {
        if (!segment.href) return segment.text;
        // Keep the decorative chain with the URL's first character when wrapping.
        const first = Array.from(segment.text)[0];
        return (
          <a
            key={index}
            className="chat-link"
            href={segment.href}
            target="_blank"
            rel="noopener noreferrer nofollow ugc"
            // The row opens message actions on click/long-press; a link only navigates.
            onClick={(event) => event.stopPropagation()}
          >
            <span className="chat-link-start">
              <Link aria-hidden="true" />
              {first}
            </span>
            {segment.text.slice(first.length)}
          </a>
        );
      })}
    </>
  );
}
