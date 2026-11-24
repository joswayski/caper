import { Fragment, useId, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Search } from "lucide-react";
import { useBackdropDismiss } from "../components/dialogBackdrop";
import { dialogKeyDown } from "../components/dialogKeyboard";
import { Dialog } from "./Dialog";
import { keyLabel, rankSwitcher, SHORTCUT_GROUPS, type Rankable } from "./shortcuts";
import "./spaces.css";

export interface SwitcherItem extends Rankable {
  key: string;
  /** What it is and where, such as "Weekend crew" for a channel or "@ana" for a person. */
  detail?: string;
  icon: ReactNode;
  open: () => void;
}

const SHOWN = 50;

/** Ctrl/Cmd+K: find a channel, DM, space or person by name; unread first. */
export function QuickSwitcher({ items, onClose }: { items: SwitcherItem[]; onClose: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const listId = useId();
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const results = useMemo(() => rankSwitcher(items, query).slice(0, SHOWN), [items, query]);
  const current = Math.min(active, results.length - 1);
  const backdrop = useBackdropDismiss(onClose);
  useLayoutEffect(() => {
    const dialog = ref.current!;
    const opener = document.activeElement as HTMLElement | null;
    dialog.showModal();
    dialog.querySelector("input")?.focus();
    return () => {
      dialog.close();
      if (opener?.isConnected) opener.focus({ preventScroll: true });
    };
  }, []);
  useLayoutEffect(() => {
    ref.current?.querySelector(`[data-index="${current}"]`)?.scrollIntoView({ block: "nearest" });
  }, [current]);
  const choose = (item: SwitcherItem | undefined) => {
    if (!item) return;
    onClose();
    item.open();
  };
  return (
    <dialog
      ref={ref}
      className="space-dialog quick-switcher"
      aria-label="Quick switcher"
      onKeyDown={dialogKeyDown}
      onCancel={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onClose();
      }}
      {...backdrop}
    >
      <label className="quick-switcher-field">
        <Search aria-hidden="true" />
        <span className="sr-only">Find a conversation</span>
        <input
          type="text"
          role="combobox"
          autoComplete="off"
          spellCheck={false}
          placeholder="Find a channel, DM, space or person"
          aria-expanded={results.length > 0}
          aria-controls={listId}
          aria-activedescendant={results.length ? `${listId}-${current}` : undefined}
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setActive(0);
          }}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing) return;
            if (event.key === "ArrowDown" || event.key === "ArrowUp") {
              event.preventDefault();
              if (!results.length) return;
              const step = event.key === "ArrowDown" ? 1 : -1;
              setActive((current + step + results.length) % results.length);
            } else if (event.key === "Enter") {
              event.preventDefault();
              choose(results[current]);
            }
          }}
        />
      </label>
      {results.length ? (
        <ul id={listId} role="listbox" aria-label="Conversations" className="quick-switcher-results">
          {results.map((item, index) => (
            <li
              key={item.key}
              id={`${listId}-${index}`}
              role="option"
              data-index={index}
              aria-selected={index === current}
              onMouseMove={() => index !== current && setActive(index)}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => choose(item)}
            >
              <span className="quick-switcher-icon">{item.icon}</span>
              <span className="quick-switcher-name">{item.name}</span>
              {item.detail && <small>{item.detail}</small>}
              {item.unread && (
                <>
                  <i className="quick-switcher-unread" aria-hidden="true" />
                  <span className="sr-only">, unread</span>
                </>
              )}
            </li>
          ))}
        </ul>
      ) : (
        <p className="quick-switcher-empty" role="status">
          {query.trim() ? "No matches" : "Nothing to switch to yet"}
        </p>
      )}
      <p className="quick-switcher-hint" aria-hidden="true">
        <kbd>↑</kbd>
        <kbd>↓</kbd> to choose, <kbd>{keyLabel(["enter"])[0]}</kbd> to open, <kbd>Esc</kbd> to close
      </p>
    </dialog>
  );
}

/** Ctrl/Cmd+/: every keyboard shortcut, in this platform's key names. */
export function ShortcutHelp({ onClose }: { onClose: () => void }) {
  return (
    <Dialog title="Keyboard shortcuts" dismissOnBackdrop onClose={onClose}>
      <div className="shortcut-help">
        {SHORTCUT_GROUPS.map((group) => (
          <section key={group.title}>
            <h3>{group.title}</h3>
            <dl>
              {group.shortcuts.map((shortcut) => (
                <div key={shortcut.label}>
                  <dt>{shortcut.label}</dt>
                  <dd>
                    {shortcut.keys.map((combo, index) => (
                      <Fragment key={combo.join("+")}>
                        {index > 0 && <span className="shortcut-or">/</span>}
                        <span className="shortcut-combo">
                          {keyLabel(combo).map((key) => (
                            <kbd key={key}>{key}</kbd>
                          ))}
                        </span>
                      </Fragment>
                    ))}
                  </dd>
                </div>
              ))}
            </dl>
          </section>
        ))}
      </div>
    </Dialog>
  );
}
