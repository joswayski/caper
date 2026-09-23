import { ChatHistoryError, loadChatHistory } from "../chat/client.ts";
import { getSpace, SpacesApiError, type SpaceDetail } from "./client.ts";
import type { GeneralChatHistory } from "../chat/types.ts";

export interface PreparedSpace {
  detail: SpaceDetail;
  channelId?: string;
  history?: GeneralChatHistory;
  historyError?: string;
}

// Scoped to one mounted Spaces page. Speculation only fetches
// reads: no voice, sockets, identity sessions, or persistent private-data cache.
export function createSpaceNavigation() {
  const entries = new Map<string, { expires: number; result: Promise<PreparedSpace> }>();
  const visited = new Map<string, PreparedSpace>();
  let demo: SpaceDetail | undefined;
  const peek = (spaceId: string, channelId?: string) => {
    const match = [...visited.values()].reverse().find((view) => view.detail.space.id === spaceId && (!channelId || view.channelId === channelId));
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
  const prepare = (spaceId: string, channelId?: string) => {
    const key = `${spaceId}:${channelId ?? ""}`;
    const cached = entries.get(key);
    if (cached && cached.expires > Date.now()) return cached.result;
    const result = (async () => {
      const detail = spaceId === demo?.space.id ? demo : await getSpace(spaceId);
      const previous = peek(spaceId, channelId);
      if (previous?.channelId && !detail.channels.some((channel) => channel.id === previous.channelId)) {
        forget(spaceId);
        throw new SpacesApiError(404, "This channel is no longer accessible.");
      }
      const channel = detail.channels.find((item) => item.id === (channelId ?? previous?.channelId)) ?? detail.channels[0];
      let history: GeneralChatHistory | undefined;
      let historyError: string | undefined;
      try {
        // Visited channels resume live replay from their saved cursor. Keep
        // their messages on screen instead of fetching the first page again.
        history = previous?.history ?? (channel ? await loadChatHistory(channel.id) : undefined);
      } catch (error) {
        if (error instanceof ChatHistoryError && [401, 403, 404].includes(error.status)) throw error;
        // Messaging being unavailable must not hide space settings/navigation.
        historyError = error instanceof Error ? error.message : "Messages are unavailable.";
      }
      if (history && history.space.id !== detail.space.id) throw new Error("The chat service returned the wrong space.");
      return { detail, channelId: channel?.id, history, historyError };
    })();
    entries.delete(key);
    entries.set(key, { expires: Date.now() + 5_000, result });
    if (entries.size > 4) entries.delete(entries.keys().next().value!);
    void result.catch((error) => {
      if ((error instanceof SpacesApiError || error instanceof ChatHistoryError) && [401, 403, 404].includes(error.status)) forget(spaceId);
      if (entries.get(key)?.result === result) entries.delete(key);
    });
    return result;
  };
  return {
    prepare,
    peek,
    remember,
    forget,
    setDemo(history: GeneralChatHistory) {
      demo = {
        space: { ...history.space, ownerId: "", demo: true },
        channels: [{ ...history.channel, spaceId: history.space.id, private: false }],
        members: [],
      };
      const view = { detail: demo, channelId: history.channel.id, history };
      remember(view);
      return view;
    },
    rememberHistory(history: GeneralChatHistory) {
      const view = peek(history.space.id, history.channel.id);
      if (view) remember({ ...view, history, historyError: undefined });
    },
    take(spaceId: string, channelId?: string) {
      const result = prepare(spaceId, channelId);
      // Speculation is single use; visited snapshots live separately. Each
      // return still rechecks space/channel access before trusting metadata.
      entries.delete(`${spaceId}:${channelId ?? ""}`);
      return result;
    },
    clear() { entries.clear(); visited.clear(); },
  };
}
