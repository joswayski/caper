import { ChatHistoryError, loadChatHistory } from "../chat/client.ts";
import { getSpace, prefetchChannelUnread, SpacesApiError, type SpaceDetail } from "./client.ts";
import { attachmentsOf, attachmentView, type GeneralChatHistory } from "../chat/types.ts";
import { friendlyError } from "./errors.ts";

export interface PreparedSpace {
  detail: SpaceDetail;
  channelId?: string;
  history?: GeneralChatHistory;
  historyError?: string;
}

export interface PreparedDirect {
  history?: GeneralChatHistory;
  historyError?: string;
}

export interface SpaceLanding {
  spaceId?: string;
  channelId?: string;
}

/** Remember IDs only, per account/browser. Private messages never go in storage. */
export function rememberSpaceLanding(accountId: string, view: PreparedSpace) {
  if (view.detail.channels.find((channel) => channel.id === view.channelId)?.joined === false) return;
  try {
    localStorage.setItem(
      `caper:space-landing:${accountId}`,
      JSON.stringify({ spaceId: view.detail.space.id, channelId: view.channelId }),
    );
  } catch {
    /* Storage is optional. */
  }
}

/** Prepare the first conversation in the route loader, not after the shell mounts. */
export async function prepareSpaceLanding(
  accountId: string,
  spaces: Array<{ id: string }>,
  query: URLSearchParams,
): Promise<{ initialSelection: SpaceLanding; initialView?: PreparedSpace; initialSpaceError?: string }> {
  let saved: SpaceLanding | undefined;
  try {
    const value = JSON.parse(localStorage.getItem(`caper:space-landing:${accountId}`) ?? "null");
    if (typeof value?.spaceId === "string")
      saved = { spaceId: value.spaceId, channelId: typeof value.channelId === "string" ? value.channelId : undefined };
  } catch {
    /* Missing, corrupt or unavailable storage uses the first joined channel. */
  }
  const requested = query.get("space") ?? undefined;
  const spaceId = spaces.find((space) => space.id === (requested ?? saved?.spaceId))?.id ?? spaces[0]?.id;
  const initialSelection = { spaceId, channelId: query.get("channel") ?? undefined };
  // Invitations, DMs and feedback visits keep their existing explicit entry flow.
  if (requested && requested !== spaceId) return { initialSelection: { ...initialSelection, spaceId: requested } };
  if (!spaceId || query.has("dm") || query.has("feedback")) return { initialSelection };
  try {
    const initialView = await createSpaceNavigation().take(
      spaceId,
      initialSelection.channelId,
      !requested && saved?.spaceId === spaceId ? saved.channelId : undefined,
    );
    // Start only a bounded set of recent inline images/posters. Never wait for
    // media, download video/audio originals, or activate a chat session/socket.
    if (typeof Image !== "undefined") {
      const urls = new Set<string>();
      for (const message of initialView.history?.messages.slice(-8) ?? []) {
        for (const attachment of attachmentsOf(message)) {
          if (attachment.spoiler) continue;
          const view = attachmentView(attachment);
          const url =
            view === "image"
              ? attachment.contentType === "image/gif"
                ? attachment.url
                : (attachment.previewUrl ?? attachment.url)
              : ["video", "animated"].includes(view)
                ? attachment.previewUrl
                : undefined;
          if (url) urls.add(url);
        }
      }
      for (const url of [...urls].slice(-4)) {
        const image = new Image();
        image.src = url;
        void image.decode().catch(() => undefined);
      }
    }
    return { initialSelection: { spaceId, channelId: initialView.channelId }, initialView };
  } catch (error) {
    return { initialSelection, initialSpaceError: friendlyError(error) };
  }
}

// Like channel navigation, DMs prepare read-only history before replacing the
// displayed chat. Private snapshots live only for this mounted Spaces page.
export function createDirectNavigation() {
  const entries = new Map<string, { expires: number; controller: AbortController; result: Promise<PreparedDirect> }>();
  const visited = new Map<string, PreparedDirect>();
  const forget = (id: string) => {
    visited.delete(id);
    entries.get(id)?.controller.abort();
    entries.delete(id);
  };
  const prepare = (id: string) => {
    const cached = entries.get(id);
    if (cached && cached.expires > Date.now()) return cached;
    cached?.controller.abort();
    const controller = new AbortController();
    const result = (async () => {
      try {
        const history = await loadChatHistory(id, controller.signal);
        if (history.space.id !== "" || history.channel.direct !== true)
          throw new Error("The chat service returned the wrong conversation.");
        return { history };
      } catch (error) {
        if (controller.signal.aborted || (error instanceof ChatHistoryError && [401, 403, 404].includes(error.status)))
          throw error;
        return { historyError: error instanceof Error ? error.message : "Messages are unavailable." };
      }
    })();
    const entry = { expires: Date.now() + 5_000, controller, result };
    entries.delete(id);
    entries.set(id, entry);
    if (entries.size > 4) forget(entries.keys().next().value!);
    void result.catch(() => {
      if (entries.get(id) === entry) forget(id);
    });
    return entry;
  };
  return {
    peek: (id: string) => visited.get(id),
    prefetch(id: string) {
      if (!visited.get(id)?.history) void prepare(id).result.catch(() => undefined);
    },
    take(id: string, signal: AbortSignal): Promise<PreparedDirect> {
      const retained = visited.get(id);
      if (retained?.history) return Promise.resolve(retained);
      const entry = prepare(id);
      entries.delete(id);
      signal.addEventListener("abort", () => entry.controller.abort(), { once: true });
      return entry.result;
    },
    remember(id: string, view: PreparedDirect) {
      visited.delete(id);
      visited.set(id, view);
      if (visited.size > 20) visited.delete(visited.keys().next().value!);
    },
    rememberHistory(history: GeneralChatHistory) {
      // Cleanup must not resurrect a declined or inaccessible conversation.
      // A search window is not a latest-page snapshot: its replay cursor can
      // sit beyond messages that have not been loaded into that window.
      if (history.hasNewer) {
        forget(history.channel.id);
        return;
      }
      if (visited.has(history.channel.id)) visited.set(history.channel.id, { history });
    },
    forget,
    clear() {
      for (const entry of entries.values()) entry.controller.abort();
      entries.clear();
      visited.clear();
    },
  };
}

// Scoped to one mounted Spaces page. Speculation only fetches
// reads: no voice, sockets, identity sessions, or persistent private-data cache.
/** The error for a requested channel that was deleted or revoked. */
export const CHANNEL_GONE = "This channel is no longer accessible.";

export function createSpaceNavigation() {
  const entries = new Map<string, { expires: number; result: Promise<PreparedSpace> }>();
  const visited = new Map<string, PreparedSpace>();
  const peek = (spaceId: string, channelId?: string) => {
    const match = [...visited.values()]
      .reverse()
      .find((view) => view.detail.space.id === spaceId && (!channelId || view.channelId === channelId));
    return match;
  };
  const remember = (view: PreparedSpace) => {
    const key = `${view.detail.space.id}:${view.channelId ?? ""}`;
    visited.delete(key);
    visited.set(key, view);
    if (visited.size > 20) visited.delete(visited.keys().next().value!);
  };
  const forget = (spaceId: string) => {
    for (const key of visited.keys()) if (key.startsWith(`${spaceId}:`)) visited.delete(key);
    for (const key of entries.keys()) if (key.startsWith(`${spaceId}:`)) entries.delete(key);
  };
  // A visit (not a hover prefetch) also starts the channel's unread snapshot,
  // beside its history when it is fetched fresh.
  const prepare = (spaceId: string, channelId?: string, preferredChannelId?: string, visit = false) => {
    const key = `${spaceId}:${channelId ?? ""}`;
    const cached = entries.get(key);
    if (cached && cached.expires > Date.now()) {
      if (visit)
        void cached.result.then(
          (view) => view.channelId && prefetchChannelUnread(view.channelId),
          () => undefined,
        );
      return cached.result;
    }
    const result = (async () => {
      // The landing channel is usually known before the space detail returns
      // (the requested, last visited, or saved landing channel), so its first
      // page is requested beside the detail. It is used only when the detail
      // confirms that channel; otherwise it is dropped.
      const known = peek(spaceId, channelId);
      const guess = channelId ?? known?.channelId ?? preferredChannelId;
      const early =
        guess && known?.history?.channel.id !== guess
          ? { channelId: guess, history: loadChatHistory(guess) }
          : undefined;
      void early?.history.catch(() => undefined);
      const detail = await getSpace(spaceId);
      const previous = peek(spaceId, channelId);
      const requestedChannel = channelId ?? previous?.channelId;
      if (requestedChannel !== undefined && !detail.channels.some((channel) => channel.id === requestedChannel)) {
        forget(spaceId);
        // A link to a channel that is gone must not open another conversation
        // as if it were that one. A remembered channel nobody asked for just
        // falls through to the space's landing channel.
        if (channelId !== undefined) throw new SpacesApiError(404, CHANNEL_GONE);
      }
      const channel =
        detail.channels.find(
          (item) => item.id === requestedChannel && (channelId !== undefined || item.joined !== false),
        ) ??
        detail.channels.find((item) => item.id === preferredChannelId && item.joined !== false) ??
        detail.channels.find((item) => item.joined !== false);
      let history: GeneralChatHistory | undefined;
      let historyError: string | undefined;
      if (visit && channel) prefetchChannelUnread(channel.id);
      try {
        // Visited channels resume live replay from their saved cursor. Keep
        // their messages on screen instead of fetching the first page again.
        history =
          channel && previous?.history?.channel.id === channel.id
            ? previous.history
            : channel
              ? await (early?.channelId === channel.id ? early.history : loadChatHistory(channel.id))
              : undefined;
      } catch (error) {
        if (error instanceof ChatHistoryError && [401, 403, 404].includes(error.status)) throw error;
        // Messaging being unavailable must not hide space settings/navigation.
        historyError = error instanceof Error ? error.message : "Messages are unavailable.";
      }
      if (history && history.space.id !== detail.space.id)
        throw new Error("The chat service returned the wrong space.");
      return { detail, channelId: channel?.id, history, historyError };
    })();
    entries.delete(key);
    entries.set(key, { expires: Date.now() + 5_000, result });
    if (entries.size > 4) entries.delete(entries.keys().next().value!);
    void result.catch((error) => {
      if (
        (error instanceof SpacesApiError || error instanceof ChatHistoryError) &&
        [401, 403, 404].includes(error.status)
      )
        forget(spaceId);
      if (entries.get(key)?.result === result) entries.delete(key);
    });
    return result;
  };
  return {
    prepare,
    peek,
    remember,
    forget,
    rememberHistory(history: GeneralChatHistory) {
      const view = peek(history.space.id, history.channel.id);
      if (view) remember({ ...view, history, historyError: undefined });
    },
    take(spaceId: string, channelId?: string, preferredChannelId?: string) {
      const result = prepare(spaceId, channelId, preferredChannelId, true);
      // Speculation is single use; visited snapshots live separately. Each
      // return still rechecks space/channel access before trusting metadata.
      entries.delete(`${spaceId}:${channelId ?? ""}`);
      return result;
    },
    clear() {
      entries.clear();
      visited.clear();
    },
  };
}
