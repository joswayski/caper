import { Link } from "lucide-react";
import { linkSegments } from "./links.ts";
import FormattedText, { QuoteBlock, quoteBlocks } from "./FormattedText.tsx";
import { formattedRuns } from "./formatting.ts";

/** Message formatting and safe autolinks; mention-aware callers format their outer runs. */
export default function LinkedText({ text, formatted = true }: { text: string; formatted?: boolean }) {
  if (formatted) {
    const runs = formattedRuns(text);
    return quoteBlocks(runs).map((block) => (
      <QuoteBlock key={block.start} quote={block.quote}>
        <FormattedText runs={runs} range={block}>
          {(part) => <LinkedText text={part} formatted={false} />}
        </FormattedText>
      </QuoteBlock>
    ));
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
