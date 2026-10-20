import { useEffect, useRef, useState, type RefObject } from "react";
import {
  autoUpdate,
  flip,
  FloatingFocusManager,
  FloatingNode,
  FloatingOverlay,
  FloatingPortal,
  offset,
  shift,
  useDismiss,
  useFloating,
  useFloatingNodeId,
  useInteractions,
  useRole,
} from "@floating-ui/react";
import { Search, X } from "lucide-react";
import GifImage from "./GifImage.tsx";
import {
  gifLabel,
  klipyConfig,
  klipyPage,
  type ChatGif,
  type KlipyConfig,
  type KlipyItem,
  type KlipyKind,
} from "./klipy.ts";

const touchLayout = () => window.matchMedia("(max-width: 760px), (pointer: coarse)").matches;

/** Whether this account has GIFs; re-checked on mount so a key added later appears. */
export function useKlipy() {
  const [config, setConfig] = useState<KlipyConfig | null>(null);
  useEffect(() => {
    let live = true;
    void klipyConfig().then((value) => live && setConfig(value));
    return () => {
      live = false;
    };
  }, []);
  return config;
}

/**
 * The composer's GIF control: a KLIPY picker above the button, or a bottom
 * drawer on touch layouts. Choosing a GIF sends it at once; `onSelect` gets the
 * search that found it, for KLIPY's share report. Hidden without a KLIPY key.
 */
export default function GifButton({
  disabled,
  onSelect,
  onOpenChange,
  returnFocus,
}: {
  disabled?: boolean;
  onSelect: (gif: ChatGif, query: string) => void;
  onOpenChange?: (open: boolean) => void;
  returnFocus?: RefObject<HTMLTextAreaElement | null>;
}) {
  const config = useKlipy();
  const [open, setOpenState] = useState(false);
  const setOpen = (next: boolean) => {
    setOpenState(next);
    onOpenChange?.(next);
  };
  const [drawer, setDrawer] = useState(false);
  const nodeId = useFloatingNodeId();
  const { refs, floatingStyles, context } = useFloating({
    nodeId,
    open,
    onOpenChange: setOpen,
    placement: "top-end",
    strategy: "fixed",
    middleware: [offset(8), flip(), shift({ padding: 12, crossAxis: true })],
    whileElementsMounted: autoUpdate,
  });
  const { getReferenceProps, getFloatingProps } = useInteractions([
    useDismiss(context, { bubbles: false }),
    useRole(context),
  ]);
  if (!config) return null;

  const panel = (
    <FloatingFocusManager context={context} returnFocus={returnFocus ?? false}>
      <div
        ref={refs.setFloating}
        className={`chat-reaction-picker chat-gif-picker${drawer ? " chat-reaction-picker-drawer" : ""}`}
        style={drawer ? undefined : floatingStyles}
        aria-label="GIFs and stickers"
        {...getFloatingProps()}
      >
        {drawer && <div className="chat-drawer-handle" aria-hidden="true" />}
        <div className="chat-reaction-picker-heading">
          <strong>GIFs and stickers</strong>
          <button type="button" aria-label="Close GIF picker" onClick={() => setOpen(false)}>
            <X size={16} />
          </button>
        </div>
        <div className="chat-reaction-picker-body">
          <GifPicker
            config={config}
            ads={drawer}
            autoFocus={!drawer}
            onSelect={(gif, query) => {
              setOpen(false);
              onSelect(gif, query);
            }}
          />
        </div>
      </div>
    </FloatingFocusManager>
  );

  return (
    <FloatingNode id={nodeId}>
      <button
        ref={refs.setReference}
        type="button"
        className="chat-composer-tool chat-gif-tool"
        aria-label="Send a GIF or sticker"
        title="GIFs and stickers"
        aria-expanded={open}
        disabled={disabled}
        onMouseDown={(event) => event.preventDefault()}
        {...getReferenceProps({
          onClick: () => {
            setDrawer(touchLayout());
            setOpen(!open);
          },
        })}
      >
        <span aria-hidden="true">GIF</span>
      </button>
      {open && (
        <FloatingPortal root={refs.domReference.current?.closest<HTMLElement>(".live-scene") ?? undefined}>
          {drawer ? (
            <FloatingOverlay lockScroll className="chat-actions-overlay">
              {panel}
            </FloatingOverlay>
          ) : (
            panel
          )}
        </FloatingPortal>
      )}
    </FloatingNode>
  );
}

type Results = { key: string; items: KlipyItem[]; page: number; hasNext: boolean; loading: boolean; error?: boolean };

export function GifPicker({
  config,
  ads,
  autoFocus,
  onSelect,
}: {
  config: KlipyConfig;
  /** Touch layouts ask KLIPY for ads; it serves them only to mobile browsers. */
  ads: boolean;
  autoFocus?: boolean;
  onSelect: (gif: ChatGif, query: string) => void;
}) {
  const [kind, setKind] = useState<KlipyKind>("gif");
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState("");
  const [attempt, setAttempt] = useState(0);
  const [results, setResults] = useState<Results>();
  const grid = useRef<HTMLDivElement>(null);
  const end = useRef<HTMLDivElement>(null);
  // One controller per tab and search: switching cancels every page it started.
  const requests = useRef<AbortController>(undefined);
  const key = `${kind}\n${search}\n${attempt}`;

  useEffect(() => {
    const timer = setTimeout(() => setSearch(query.trim()), query.trim() ? 300 : 0);
    return () => clearTimeout(timer);
  }, [query]);

  const load = (page: number) => {
    const signal = requests.current?.signal;
    if (!signal || signal.aborted) return;
    setResults((current) =>
      current?.key === key
        ? { ...current, loading: true, error: false }
        : { key, items: [], page: 0, hasNext: false, loading: true },
    );
    const width = grid.current?.clientWidth ?? 320;
    klipyPage(config, kind, search, page, ads ? { width } : undefined, signal).then(
      (next) =>
        setResults((current) => ({
          key,
          items: page === 1 || current?.key !== key ? next.items : [...current.items, ...next.items],
          page,
          hasNext: next.hasNext,
          loading: false,
        })),
      () => {
        if (!signal.aborted) setResults((current) => current && { ...current, key, loading: false, error: true });
      },
    );
  };

  useEffect(() => {
    const controller = new AbortController();
    requests.current = controller;
    load(1);
    grid.current?.scrollTo({ top: 0 });
    return () => controller.abort();
  }, [key]);

  // Load the next page as the end of the grid comes into view.
  const current = results?.key === key ? results : undefined;
  const more = !!current && current.hasNext && !current.loading && !current.error;
  useEffect(() => {
    const sentinel = end.current;
    if (!sentinel || !more) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          observer.disconnect();
          load(current.page + 1);
        }
      },
      { root: grid.current, rootMargin: "240px" },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [more, current?.page, key]);

  const items = current?.items ?? [];
  return (
    <div className="chat-gif-panel">
      <div className="chat-gif-tabs" role="tablist" aria-label="Kind">
        {(["gif", "sticker"] as const).map((item) => (
          <button key={item} type="button" role="tab" aria-selected={kind === item} onClick={() => setKind(item)}>
            {item === "gif" ? "GIFs" : "Stickers"}
          </button>
        ))}
      </div>
      <div className="chat-emoji-search">
        <Search size={18} aria-hidden="true" />
        <input
          aria-label={kind === "gif" ? "Search KLIPY GIFs" : "Search KLIPY stickers"}
          placeholder="Search KLIPY"
          autoFocus={autoFocus}
          enterKeyHint="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        {query && (
          <button type="button" aria-label="Clear search" onClick={() => setQuery("")}>
            <X size={16} />
          </button>
        )}
      </div>
      <div ref={grid} className="chat-gif-grid" data-kind={kind} aria-busy={!current || current.loading}>
        {items.map((item, index) =>
          item.type === "ad" ? (
            <GifAd key={`ad:${index}`} ad={item} />
          ) : (
            <button
              key={`${item.slug}:${index}`}
              type="button"
              className="chat-gif-tile"
              title={item.title || undefined}
              aria-label={gifLabel(item.gif)}
              onClick={() => onSelect(item.gif, search)}
            >
              <GifTile item={item} />
            </button>
          ),
        )}
        <div ref={end} className="chat-gif-end" aria-hidden="true" />
        {current && !current.loading && !current.error && !items.length && (
          <p className="chat-emoji-empty" role="status">
            No results. Try another search.
          </p>
        )}
        {current?.error && (
          <p className="chat-gif-error" role="alert">
            {kind === "gif" ? "GIFs" : "Stickers"} couldn't load.{" "}
            <button type="button" onClick={() => setAttempt((value) => value + 1)}>
              Retry
            </button>
          </p>
        )}
      </div>
      <small className="chat-gif-attribution">Powered by KLIPY</small>
    </div>
  );
}

function GifTile({ item }: { item: Extract<KlipyItem, { type: "item" }> }) {
  const [failed, setFailed] = useState(false);
  if (failed) return <span className="chat-gif-tile-failed">GIF unavailable</span>;
  return (
    <span className="chat-gif-tile-media" data-still={item.tile.still ? true : undefined}>
      <GifImage
        src={item.tile.url}
        alt=""
        loading="lazy"
        decoding="async"
        referrerPolicy="origin"
        width={item.tile.width}
        height={item.tile.height}
        onError={() => setFailed(true)}
      />
      {/* Reduced motion swaps in KLIPY's own still where it has one. */}
      {item.tile.still && <img className="chat-gif-tile-still" src={item.tile.still} alt="" loading="lazy" />}
    </span>
  );
}

/** A KLIPY ad, isolated from Caper: a sandboxed frame with no same-origin
 * access, whose links open in a new tab. */
function GifAd({ ad }: { ad: Extract<KlipyItem, { type: "ad" }> }) {
  return (
    <div className="chat-gif-ad">
      <small>Sponsored</small>
      <iframe
        title="Sponsored"
        sandbox="allow-scripts allow-popups allow-popups-to-escape-sandbox"
        referrerPolicy="origin"
        loading="lazy"
        srcDoc={`<!doctype html><base target="_blank"><style>html,body{margin:0}</style>${ad.html}`}
        style={{ width: ad.width ?? "100%", maxWidth: "100%", height: ad.height ?? 100 }}
      />
    </div>
  );
}
