import { useSyncExternalStore } from "react";
import { blockAccount, listBlocks, unblockAccount, type BlockedAccount } from "./client.ts";

// One list per tab, shared by the sidebar, every chat and the settings dialog.
let blocks: BlockedAccount[] = [];
let blockedIds: ReadonlySet<string> = new Set();
const listeners = new Set<() => void>();
const EMPTY: BlockedAccount[] = [];
const EMPTY_IDS: ReadonlySet<string> = new Set();

function publish(next: BlockedAccount[]) {
  blocks = next;
  blockedIds = new Set(next.map((account) => account.id));
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

export async function refreshBlocks() {
  publish((await listBlocks()).blocks);
}

/** A chat author has no username, so the list is re-read after the change. */
export async function block(account: BlockedAccount) {
  await blockAccount(account.id);
  publish([account, ...blocks.filter((item) => item.id !== account.id)]);
  void refreshBlocks().catch(() => undefined);
}

export async function unblock(id: string) {
  await unblockAccount(id);
  publish(blocks.filter((item) => item.id !== id));
}

export function useBlocks() {
  return useSyncExternalStore(subscribe, () => blocks, () => EMPTY);
}

export function useBlockedIds() {
  return useSyncExternalStore(subscribe, () => blockedIds, () => EMPTY_IDS);
}
