import { linkSegments } from "./links.ts";

/** Plain message text with http(s) and www. links; see links.ts for the shared rules. */
export default function LinkedText({ text }: { text: string }) {
  return (
    <>
      {linkSegments(text).map((segment, index) =>
        segment.href ? (
          <a
            key={index}
            className="chat-link"
            href={segment.href}
            target="_blank"
            rel="noopener noreferrer nofollow ugc"
            // The row opens message actions on click/long-press; a link only navigates.
            onClick={(event) => event.stopPropagation()}
          >
            {segment.text}
          </a>
        ) : (
          segment.text
        ),
      )}
    </>
  );
}
