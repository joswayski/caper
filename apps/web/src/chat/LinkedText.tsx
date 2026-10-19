import { ExternalLink } from "lucide-react";
import { linkSegments } from "./links.ts";

/** Plain message text with http(s) and www. links; see links.ts for the shared rules. */
export default function LinkedText({ text }: { text: string }) {
  return (
    <>
      {linkSegments(text).map((segment, index) => {
        if (!segment.href) return segment.text;
        // Keep the icon with the URL's last code point, even when the URL wraps.
        const last = Array.from(segment.text).at(-1)!;
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
            {segment.text.slice(0, -last.length)}
            <span className="chat-link-end">
              {last}
              <ExternalLink aria-hidden="true" />
            </span>
          </a>
        );
      })}
    </>
  );
}
