import { Play } from "lucide-react";
import { useState } from "react";
import LinkedText from "./LinkedText.tsx";
import type { ChatEmbed, EmbedBlock } from "./embeds.ts";

const MAX_WIDTH = 400;
const MAX_HEIGHT = 300;
const LINK = { target: "_blank", rel: "noopener noreferrer nofollow ugc" } as const;

/** Display size that reserves layout space before media loads; media of
 * unknown size gets the full width at 16:9. */
export function fit(width?: number, height?: number) {
  if (!width || !height) return { width: MAX_WIDTH, aspectRatio: "16 / 9" };
  const scale = Math.min(1, MAX_WIDTH / width, MAX_HEIGHT / height);
  return { width: Math.round(width * scale), aspectRatio: `${width} / ${height}` };
}

function reducedMotion() {
  return typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** Each provider's own player, loaded only after Play. The browser keeps it in
 * its own origin and process; it gets no access to Caper. */
export function playerUrl(block: Extract<EmbedBlock, { type: "player" }>, host: string) {
  const id = encodeURIComponent(block.id);
  if (block.provider === "youtube") {
    const start = block.start ? `&start=${Math.floor(block.start)}` : "";
    return `https://www.youtube-nocookie.com/embed/${id}?autoplay=1&playsinline=1${start}`;
  }
  if (block.provider === "vimeo") return `https://player.vimeo.com/video/${id}?autoplay=1`;
  const [kind, value = ""] = block.id.split(":");
  const target = encodeURIComponent(value);
  const parent = `parent=${encodeURIComponent(host)}&autoplay=true`;
  if (kind === "clip") return `https://clips.twitch.tv/embed?clip=${target}&${parent}`;
  if (kind === "video") return `https://player.twitch.tv/?video=v${target}&${parent}`;
  return `https://player.twitch.tv/?channel=${target}&${parent}`;
}

function Player({ block, title }: { block: Extract<EmbedBlock, { type: "player" }>; title: string }) {
  const [playing, setPlaying] = useState(false);
  const size = fit(block.width, block.height);
  return (
    <div className="chat-embed-media chat-embed-player" style={size}>
      {playing ? (
        <iframe
          src={playerUrl(block, location.hostname)}
          title={title}
          allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
          allowFullScreen
          referrerPolicy="strict-origin-when-cross-origin"
          sandbox="allow-scripts allow-same-origin allow-presentation allow-popups allow-popups-to-escape-sandbox"
        />
      ) : (
        <button type="button" onClick={() => setPlaying(true)} aria-label={`Play ${title}`}>
          {block.poster && <img src={block.poster} alt="" loading="lazy" referrerPolicy="no-referrer" />}
          <span className="chat-embed-play" aria-hidden="true">
            <Play size={22} fill="currentColor" />
          </span>
        </button>
      )}
    </div>
  );
}

function Video({ block, title }: { block: Extract<EmbedBlock, { type: "video" }>; title: string }) {
  const size = fit(block.width, block.height);
  if (block.gif) {
    const still = reducedMotion();
    return (
      <div className="chat-embed-media" style={size}>
        <video
          src={block.url}
          poster={block.poster}
          autoPlay={!still}
          controls={still}
          loop
          muted
          playsInline
          preload="metadata"
          aria-label={title}
        />
      </div>
    );
  }
  return (
    <div className="chat-embed-media" style={size}>
      <video src={block.url} poster={block.poster} controls playsInline preload="none" aria-label={title} />
    </div>
  );
}

function footerTime(timestamp?: string) {
  if (!timestamp) return null;
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return null;
  return (
    <time dateTime={date.toISOString()} suppressHydrationWarning>
      {date.toLocaleDateString(undefined, { dateStyle: "medium" })}
    </time>
  );
}

function EmbedCard({ embed }: { embed: ChatEmbed }) {
  const heading =
    embed.blocks.find((block) => block.type === "title")?.text ??
    embed.blocks.find((block) => block.type === "author")?.name ??
    embed.site.name;
  const thumbnail = embed.blocks.find(
    (block): block is Extract<EmbedBlock, { type: "images" }> => block.type === "images" && !!block.thumbnail,
  );
  const body = embed.blocks.map((block, index) => {
    switch (block.type) {
      case "author":
        return (
          <div key={index} className="chat-embed-author">
            {block.icon && <img src={block.icon} alt="" loading="lazy" referrerPolicy="no-referrer" />}
            {block.url ? (
              <a href={block.url} {...LINK}>
                <strong>{block.name}</strong>
                {block.handle && <span>{block.handle}</span>}
              </a>
            ) : (
              <span>
                <strong>{block.name}</strong>
                {block.handle && <span>{block.handle}</span>}
              </span>
            )}
          </div>
        );
      case "title":
        return (
          <p key={index} className="chat-embed-title">
            {block.url ? (
              <a href={block.url} {...LINK}>
                {block.text}
              </a>
            ) : (
              block.text
            )}
          </p>
        );
      case "text":
        return (
          <p key={index} className="chat-embed-text">
            <LinkedText text={block.text} />
          </p>
        );
      case "images":
        if (block.thumbnail) return null;
        return (
          <div key={index} className="chat-embed-images" data-count={block.items.length}>
            {block.items.map((image, item) => (
              <a
                key={item}
                href={image.url}
                {...LINK}
                className="chat-embed-media"
                style={block.items.length === 1 ? fit(image.width, image.height) : undefined}
              >
                <img src={image.url} alt={image.alt ?? ""} loading="lazy" referrerPolicy="no-referrer" />
              </a>
            ))}
          </div>
        );
      case "video":
        return <Video key={index} block={block} title={heading} />;
      case "player":
        return <Player key={index} block={block} title={heading} />;
      case "footer":
        return (
          <p key={index} className="chat-embed-footer">
            {block.text}
            {block.text && block.timestamp && " · "}
            {footerTime(block.timestamp)}
          </p>
        );
      default:
        return null;
    }
  });
  return (
    <article className="chat-embed" aria-label={`${embed.site.name}: ${heading}`}>
      <div className="chat-embed-body">
        <a className="chat-embed-site" href={embed.url} {...LINK}>
          {embed.site.name}
          {embed.site.name !== embed.site.domain && <span>{embed.site.domain}</span>}
        </a>
        {body}
      </div>
      {thumbnail && (
        <a className="chat-embed-thumbnail" href={embed.url} {...LINK}>
          <img
            src={thumbnail.items[0].url}
            alt={thumbnail.items[0].alt ?? ""}
            loading="lazy"
            referrerPolicy="no-referrer"
          />
        </a>
      )}
    </article>
  );
}

/** A message's link preview cards, drawn from blocks; unknown blocks are skipped. */
export function MessageEmbeds({ embeds, onRemove }: { embeds: ChatEmbed[]; onRemove?: () => void }) {
  if (!embeds.length) return null;
  return (
    // The row opens message actions on click; playing or following a card only does that.
    <div className="chat-embeds" onClick={(event) => event.stopPropagation()}>
      {embeds.map((embed) => (
        <EmbedCard key={embed.url} embed={embed} />
      ))}
      {onRemove && (
        <button type="button" className="chat-embed-remove" onClick={onRemove} aria-label="Remove link preview">
          ×
        </button>
      )}
    </div>
  );
}
