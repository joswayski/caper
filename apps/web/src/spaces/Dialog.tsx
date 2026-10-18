import { useId, useLayoutEffect, useRef, type ReactNode } from "react";
import { X } from "lucide-react";
import { useBackdropDismiss } from "../components/dialogBackdrop";
import { dialogKeyDown } from "../components/dialogKeyboard";

export function Dialog({
  title,
  titleIcon,
  description,
  onClose,
  children,
  width = "standard",
  dismissOnBackdrop = false,
}: {
  title: string;
  titleIcon?: ReactNode;
  description?: string;
  onClose: () => void;
  children: ReactNode;
  width?: "standard" | "wide";
  dismissOnBackdrop?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const backdrop = useBackdropDismiss(onClose, dismissOnBackdrop);
  useLayoutEffect(() => {
    const dialog = ref.current!;
    const opener = document.activeElement as HTMLElement | null;
    dialog.showModal();
    dialog.querySelector<HTMLElement>("[data-initial-focus]")?.focus();
    return () => {
      dialog.close();
      if (opener?.isConnected) opener.focus();
    };
  }, []);
  return (
    <dialog
      ref={ref}
      className={`space-dialog ${width === "wide" ? "space-dialog-wide" : ""}`}
      aria-labelledby={titleId}
      onKeyDown={dialogKeyDown}
      onCancel={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onClose();
      }}
      {...backdrop}
    >
      <header>
        <div>
          <h2 id={titleId}>
            {titleIcon}
            {title}
          </h2>
          {description && <p>{description}</p>}
        </div>
        <button type="button" aria-label={`Close ${title}`} onClick={onClose}>
          <X aria-hidden="true" />
        </button>
      </header>
      {children}
    </dialog>
  );
}
