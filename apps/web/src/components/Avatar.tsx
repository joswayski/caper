import { useSyncExternalStore } from "react";
import { avatarUrl } from "../account/avatar";
import { getRememberedAccount, subscribeAccount } from "../account/client";
import { withPicture, type PersonPicture } from "../account/pictures";
import { getProfiles, getServerProfiles, subscribeProfiles } from "../account/profiles";
import Picture from "./Picture";

/** First character of a name, whole even when it is an emoji or other surrogate pair. */
export function initial(name: string) {
  return (Array.from(name.trim())[0] ?? "").toUpperCase();
}

/** Who an avatar shows: their saved mascot and, optionally, a custom picture. */
export interface AvatarPerson extends PersonPicture {
  /** The account ID: the signed-in account's saved picture, or a live update, wins over older snapshots. */
  id?: string;
}

/** Decorative; the adjacent account/member/author name supplies the label. */
export default function Avatar({ person, name }: { person?: AvatarPerson | null; name: string }) {
  const account = useSyncExternalStore(subscribeAccount, getRememberedAccount, () => undefined);
  const profiles = useSyncExternalStore(subscribeProfiles, getProfiles, getServerProfiles);
  const latest = person?.id ? profiles.get(person.id) : undefined;
  const shown: AvatarPerson | null | undefined =
    person?.id && person.id === account?.id
      ? {
          avatarId: account.avatarId ?? person.avatarId,
          avatarAsset: account.avatarAsset,
          avatarUrl: account.avatarUrl,
        }
      : person && latest
        ? withPicture(person, latest)
        : person;
  const url = avatarUrl(shown?.avatarId);
  return (
    <span
      aria-hidden="true"
      data-avatar-id={url ? shown?.avatarId : undefined}
      data-avatar-asset={shown?.avatarAsset || undefined}
      style={{
        position: "relative",
        display: "grid",
        placeItems: "center",
        width: "100%",
        height: "100%",
        borderRadius: "inherit",
        backgroundColor: url ? undefined : "var(--surface-composer)",
        backgroundImage: url ? `url('${url}')` : undefined,
        backgroundSize: "100% 100%",
        backgroundRepeat: "no-repeat",
      }}
    >
      {url ? null : initial(name)}
      <Picture asset={shown?.avatarAsset} url={shown?.avatarUrl} />
    </span>
  );
}
