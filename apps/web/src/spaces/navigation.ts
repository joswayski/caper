import { loadChatHistory } from "../chat/client.ts";
import { getSpace, type SpaceDetail } from "./client.ts";
import type { GeneralChatHistory } from "../chat/types.ts";

export interface PreparedSpace {
  detail: SpaceDetail;
  channelId?: string;
  history?: GeneralChatHistory;
  historyError?: string;
}

// Scoped to one mounted, authenticated Spaces page. Speculation only fetches
// reads: no voice, sockets, identity sessions, or persistent private-data cache.
export function createSpaceNavigation() {
  const entries = new Map<string, { expires: number; result: Promise<PreparedSpace> }>();
  const prepare = (spaceId: string, channelId?: string) => {
    const key = `${spaceId}:${channelId ?? ""}`;
    const cached = entries.get(key);
    if (cached && cached.expires > Date.now()) return cached.result;
    const result = (async () => {
      const detail = await getSpace(spaceId);
      const channel = detail.channels.find((item) => item.id === channelId) ?? detail.channels[0];
      let history: GeneralChatHistory | undefined;
      let historyError: string | undefined;
      try {
        history = channel ? await loadChatHistory(channel.id) : undefined;
      } catch (error) {
        // Messaging being unavailable must not hide space settings/navigation.
        historyError = error instanceof Error ? error.message : "Messages are unavailable.";
      }
      if (history && history.space.id !== detail.space.id) throw new Error("The chat service returned the wrong space.");
      return { detail, channelId: channel?.id, history, historyError };
    })();
    entries.delete(key);
    entries.set(key, { expires: Date.now() + 5_000, result });
    if (entries.size > 4) entries.delete(entries.keys().next().value!);
    void result.catch(() => {
      if (entries.get(key)?.result === result) entries.delete(key);
    });
    return result;
  };
  return {
    prepare,
    take(spaceId: string, channelId?: string) {
      const result = prepare(spaceId, channelId);
      // Single use: revisiting always reads current permissions/messages.
      entries.delete(`${spaceId}:${channelId ?? ""}`);
      return result;
    },
    clear() { entries.clear(); },
  };
}
