import { Check } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import Avatar from "../components/Avatar";
import { Dialog } from "../spaces/Dialog";
import {
  AccountApiError,
  getAccount,
  getAvatarHistory,
  selectAvatar,
  type Account,
  type PreviousPicture,
} from "./client";

/** A local choice until Save: browsing old pictures must never change the account. */
export default function AvatarHistory({ account, onClose }: { account: Account; onClose: () => void }) {
  const [pictures, setPictures] = useState<PreviousPicture[]>([]);
  const [selected, setSelected] = useState<PreviousPicture | undefined>(
    account.avatarAsset ? { avatarAsset: account.avatarAsset, avatarUrl: account.avatarUrl ?? undefined } : undefined,
  );
  const [cursor, setCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [loadError, setLoadError] = useState<string>();
  const [saveError, setSaveError] = useState<string>();
  const [stale, setStale] = useState(false);
  const request = useRef(0);
  const pending = useRef(false);
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
      if (pending.current) window.history.pushState(entry, "");
      else onClose();
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

  const save = async () => {
    if (!selected || selected.avatarAsset === account.avatarAsset || pending.current || loading || stale) return;
    pending.current = true;
    setSaving(true);
    setSaveError(undefined);
    const attempt = ++request.current;
    try {
      await selectAvatar(selected.avatarAsset, account.avatarAsset ?? null);
      if (attempt === request.current) onClose();
    } catch (error) {
      if (attempt !== request.current) return;
      const conflict = error instanceof AccountApiError && error.status === 409;
      setStale(conflict);
      setSaveError(
        conflict
          ? "Your picture changed elsewhere. Close and reopen to refresh."
          : error instanceof AccountApiError && error.status === 404
            ? "That picture is no longer available. Choose another picture."
            : "Your picture couldn’t be saved. Try again.",
      );
      if (conflict) await getAccount().catch(() => undefined);
    } finally {
      pending.current = false;
      if (attempt === request.current) setSaving(false);
    }
  };

  return (
    <Dialog
      title="Previous pictures"
      width="wide"
      onClose={() => {
        if (!pending.current) onClose();
      }}
      closeDisabled={saving}
      dismissOnBackdrop={!saving}
    >
      <div className="avatar-history" aria-busy={loading || saving}>
        <p className="avatar-history-hint">Choose a picture to preview. Save when it looks right.</p>
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
                  disabled={saving}
                  onClick={() => {
                    setSelected(picture);
                    setSaveError(undefined);
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
              <button type="button" className="secondary" disabled={saving} onClick={() => void load(cursor)}>
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
        {saveError && (
          <p className="picture-settings-error" role="alert">
            {saveError}
          </p>
        )}
        <footer className="space-dialog-actions">
          <button type="button" className="secondary" disabled={saving} onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="primary"
            disabled={saving || loading || stale || !selected || selected.avatarAsset === account.avatarAsset}
            onClick={() => void save()}
          >
            {saving ? "Saving…" : "Save picture"}
          </button>
        </footer>
      </div>
    </Dialog>
  );
}
