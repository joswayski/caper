import { Check } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import Avatar from "../components/Avatar";
import { Dialog } from "../spaces/Dialog";
import { getAvatarHistory, type Account, type PreviousPicture } from "./client";

/** A local choice until Save: browsing old pictures must never change the account. */
export default function AvatarHistory({
  account,
  onClose,
  onSelect,
}: {
  account: Account;
  onClose: () => void;
  onSelect: (picture: PreviousPicture) => void;
}) {
  const [pictures, setPictures] = useState<PreviousPicture[]>([]);
  const [selected, setSelected] = useState<PreviousPicture | undefined>(
    account.avatarAsset ? { avatarAsset: account.avatarAsset, avatarUrl: account.avatarUrl ?? undefined } : undefined,
  );
  const [cursor, setCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string>();
  const request = useRef(0);
  const load = useCallback(async (after?: string | null) => {
    const attempt = ++request.current;
    setLoading(true);
    setLoadError(undefined);
    try {
      const page = await getAvatarHistory(after);
      if (attempt !== request.current) return;
      setPictures((previous) =>
        after
          ? [
              ...previous,
              ...page.pictures.filter((item) => !previous.some((old) => old.avatarAsset === item.avatarAsset)),
            ]
          : page.pictures,
      );
      setCursor(page.nextCursor);
      setLoaded(true);
    } catch {
      if (attempt === request.current) setLoadError("Couldn’t load your pictures. Try again.");
    } finally {
      if (attempt === request.current) setLoading(false);
    }
  }, []);
  useEffect(() => {
    void load();
    return () => {
      request.current++;
    };
  }, [load]);
  // Mobile Back dismisses this chooser, not the profile underneath it.
  useEffect(() => {
    if (!window.matchMedia("(pointer: coarse), (max-width: 600px)").matches) return;
    const key = crypto.randomUUID();
    const entry = { ...window.history.state, caperAvatarHistory: key };
    if (window.history.state?.caperAvatarHistory) window.history.replaceState(entry, "");
    else window.history.pushState(entry, "");
    const back = () => {
      if (window.history.state?.caperAvatarHistory === key) return;
      onClose();
    };
    window.addEventListener("popstate", back);
    return () => {
      window.removeEventListener("popstate", back);
      // As in MediaViewer, let a same-commit remount take over this entry first.
      setTimeout(() => {
        if (window.history.state?.caperAvatarHistory === key) window.history.back();
      });
    };
  }, [onClose]);

  return (
    <Dialog title="Previous pictures" width="wide" onClose={onClose} dismissOnBackdrop>
      <div className="avatar-history" aria-busy={loading}>
        <p className="avatar-history-hint">Choose a picture for your profile draft. Save profile to publish it.</p>
        <div className="avatar-history-body">
          <div className="avatar-history-library">
            <div className="avatar-history-grid" role="group" aria-label="Saved profile pictures">
              {pictures.map((picture, index) => (
                <button
                  key={picture.avatarAsset}
                  type="button"
                  className="avatar-history-choice"
                  aria-label={`${picture.avatarAsset === account.avatarAsset ? "Current picture" : "Previous picture"} ${index + 1}`}
                  aria-pressed={selected?.avatarAsset === picture.avatarAsset}
                  onClick={() => {
                    setSelected(picture);
                  }}
                >
                  <span className="avatar-history-thumbnail">
                    <Avatar person={picture} name="Picture" />
                    {selected?.avatarAsset === picture.avatarAsset && (
                      <span className="avatar-history-check">
                        <Check size={14} aria-hidden="true" />
                      </span>
                    )}
                  </span>
                  <span className="avatar-history-current">
                    {picture.avatarAsset === account.avatarAsset ? "Current" : "\u00a0"}
                  </span>
                </button>
              ))}
            </div>
            {loading && <p role="status">Loading pictures…</p>}
            {loaded && !pictures.length && <p>No previous pictures yet. Pictures you save will appear here.</p>}
            {loadError && (
              <p className="picture-settings-error" role="alert">
                {loadError}
              </p>
            )}
            {!loading && (loadError || cursor) && (
              <button type="button" className="secondary" onClick={() => void load(cursor)}>
                {loadError ? "Retry" : "Load more"}
              </button>
            )}
          </div>
          <div className="avatar-history-preview" aria-label="Picture preview">
            <div className="avatar-history-face">
              <Avatar person={selected ?? { avatarId: account.avatarId }} name={account.displayName ?? "You"} />
            </div>
            <p>{selected ? "Preview" : "Select a picture"}</p>
          </div>
        </div>
        <footer className="space-dialog-actions">
          <button type="button" className="secondary" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="primary"
            disabled={loading || !selected || selected.avatarAsset === account.avatarAsset}
            onClick={() => selected && onSelect(selected)}
          >
            Use picture
          </button>
        </footer>
      </div>
    </Dialog>
  );
}
