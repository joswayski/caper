import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { Minimize2, X } from "lucide-react";
import type { Account } from "../account/client";
import type { GeneralChatHistory } from "../chat/types";
import Call from "../pages/Call";
import LivePreview from "./LivePreview";
import { attachLiveMotion } from "./liveMotion";
import "./live-window.css";

export type LiveStatus = "live" | "connecting" | "preview";

type LiveWindowProps = {
  account: Account | null;
  history: GeneralChatHistory | null;
  active: boolean;
  onActiveChange: (active: boolean) => void;
  onStatusChange?: (status: LiveStatus) => void;
};

/**
 * The homepage's 3D window. With the public channel available it contains the
 * real #general room (chat, voice roster and controls); opening it turns the
 * window into the app. Otherwise it plays a labeled preview.
 */
export default function LiveWindow({ account, history, active, onActiveChange, onStatusChange }: LiveWindowProps) {
  const live = !!history;
  const stageRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<HTMLDivElement>(null);
  const activatorRef = useRef<HTMLButtonElement>(null);
  const controller = useRef<ReturnType<typeof attachLiveMotion> | undefined>(undefined);
  const activeRef = useRef(active);
  const change = useRef(onActiveChange);
  change.current = onActiveChange;
  const [ready, setReady] = useState(false);
  const [sheet, setSheet] = useState(false);
  const [engaged, setEngaged] = useState(false);
  const [online, setOnline] = useState(false);
  const [touched, setTouched] = useState(false);
  const status: LiveStatus = !live ? "preview" : online ? "live" : "connecting";

  useLayoutEffect(() => {
    const stage = stageRef.current;
    const scene = sceneRef.current;
    if (!stage || !scene) return;
    controller.current = attachLiveMotion(stage, scene, {
      bounds: () => stage.closest<HTMLElement>("[data-live-bounds]"),
      activate: () => { if (live) change.current(true); },
      deactivate: () => change.current(false),
      interacted: () => setTouched(true),
    });
    setReady(true);
    return () => { controller.current?.detach(); controller.current = undefined; };
  }, [live]);

  useEffect(() => { onStatusChange?.(status); }, [status, onStatusChange]);

  useEffect(() => {
    const motion = controller.current;
    if (!motion) return;
    const opening = active && !activeRef.current;
    activeRef.current = active;
    motion.setActive(active);
    const fullScreen = active && motion.mode === "sheet";
    setSheet(fullScreen);
    if (active) setEngaged(true);
    document.documentElement.classList.toggle("live-sheet-open", fullScreen);
    if (opening) {
      if (fullScreen) sceneRef.current?.focus({ preventScroll: true });
      else {
        // Bring the hero into view, then put the cursor in the composer.
        const bounds = stageRef.current?.closest<HTMLElement>("[data-live-bounds]");
        const top = bounds?.getBoundingClientRect().top ?? 0;
        if (Math.abs(top) > 4) window.scrollTo({ top: window.scrollY + top, behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
        setTimeout(() => document.getElementById("chat-message")?.focus({ preventScroll: true }), 420);
      }
    } else if (!active) {
      activatorRef.current?.focus({ preventScroll: true });
    }
    return () => document.documentElement.classList.remove("live-sheet-open");
  }, [active]);

  useEffect(() => {
    if (!active) return;
    // Crossing the expand breakpoint while open switches between the two forms.
    const resized = () => {
      const fullScreen = controller.current?.mode === "sheet";
      setSheet(fullScreen);
      document.documentElement.classList.toggle("live-sheet-open", fullScreen);
    };
    window.addEventListener("resize", resized);
    return () => window.removeEventListener("resize", resized);
  }, [active]);

  return (
    <div className="live-stage" ref={stageRef} data-ready={ready ? "" : undefined} data-sheet={sheet ? "" : undefined} data-active={active ? "" : undefined}>
      <div className="live-glow" aria-hidden="true" />
      <div
        className="live-scene"
        ref={sceneRef}
        tabIndex={-1}
        data-active={active ? "" : undefined}
        role={sheet ? "dialog" : undefined}
        aria-modal={sheet ? true : undefined}
        aria-label={sheet ? "Caper #general" : undefined}
      >
        <div className="live-shadow" aria-hidden="true" />
        {[5, 4, 3, 2, 1].map((depth) => <div key={depth} className="live-slab" style={{ "--z": -depth * 5 } as CSSProperties} aria-hidden="true" />)}
        <div className="live-window">
          <div className="live-titlebar">
            <span className="live-lights" aria-hidden="true"><i /><i /><i /></span>
            <span className="live-wordmark" aria-hidden="true">caper<span>.</span></span>
            <span className="live-location">#general</span>
            <span className="live-status" data-status={status} role="status">
              <i aria-hidden="true" />{status === "live" ? "Live" : status === "connecting" ? "Connecting…" : "Preview"}
            </span>
            {active && <button className="live-close" type="button" data-live-control onClick={() => onActiveChange(false)} aria-label={sheet ? "Close #general" : "Minimize #general"}>
              {sheet ? <X aria-hidden="true" /> : <Minimize2 aria-hidden="true" />}
            </button>}
          </div>
          <div className="live-app" inert={!active}>
            {live
              ? <Call embedded engaged={engaged} initialAccount={account ?? undefined} initialHistory={history} onChatOnlineChange={setOnline} />
              : <LivePreview />}
          </div>
        </div>
        {live && !active && <button
          ref={activatorRef}
          className="live-activator"
          type="button"
          data-live-activator
          aria-label="Open the public #general channel. Arrow keys tilt the window."
          onClick={() => onActiveChange(true)}
        >
          <span className="live-invite" data-touched={touched ? "" : undefined}>
            <i aria-hidden="true" />
            <span className="live-invite-fine">Click to join</span>
            <span className="live-invite-coarse">Tap to join</span>
          </span>
        </button>}
      </div>
    </div>
  );
}
