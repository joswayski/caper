import { useEffect, useRef, useState } from "react";

interface TurnstileOptions {
  sitekey: string;
  action: string;
  theme: "dark";
  size: "flexible" | "compact";
  appearance: "interaction-only";
  "response-field": false;
  callback: (token: string) => void;
  "error-callback": () => boolean;
  "expired-callback": () => void;
  "timeout-callback": () => void;
  "before-interactive-callback": () => void;
  "after-interactive-callback": () => void;
}

declare global {
  interface Window {
    turnstile?: {
      render: (container: HTMLElement, options: TurnstileOptions) => string;
      remove: (id: string) => void;
    };
  }
}

export default function Turnstile({ siteKey, resetKey, onToken }: {
  siteKey: string;
  resetKey: number;
  onToken: (token: string | undefined) => void;
}) {
  const container = useRef<HTMLDivElement>(null);
  const [failed, setFailed] = useState(false);
  const [interactive, setInteractive] = useState(false);
  const [compact, setCompact] = useState(false);
  const [retry, setRetry] = useState(0);

  useEffect(() => {
    const media = matchMedia("(max-width: 339px)");
    const update = () => setCompact(media.matches);
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);

  useEffect(() => {
    let disposed = false;
    let widget: string | undefined;
    onToken(undefined);
    setFailed(false);
    setInteractive(false);
    const fail = () => {
      if (!disposed) { onToken(undefined); setFailed(true); }
      return true;
    };
    const render = () => {
      if (disposed || widget !== undefined || !container.current || !window.turnstile) return;
      try {
        widget = window.turnstile.render(container.current, {
          sitekey: siteKey,
          action: "login_email",
          theme: "dark",
          size: compact ? "compact" : "flexible",
          appearance: "interaction-only",
          "response-field": false,
          callback: (token) => { if (!disposed) { setFailed(false); onToken(token); } },
          "error-callback": fail,
          "expired-callback": () => { if (!disposed) onToken(undefined); },
          "timeout-callback": () => { if (!disposed) onToken(undefined); },
          "before-interactive-callback": () => { if (!disposed) setInteractive(true); },
          "after-interactive-callback": () => { if (!disposed) setInteractive(false); },
        });
      } catch { fail(); }
    };
    let script = document.querySelector<HTMLScriptElement>("#caper-turnstile");
    if (script?.dataset.failed) { script.remove(); script = null; }
    const newScript = !script;
    script ??= document.createElement("script");
    const scriptError = () => { script.dataset.failed = "true"; fail(); };
    script.addEventListener("load", render);
    script.addEventListener("error", scriptError);
    if (newScript) {
      script.id = "caper-turnstile";
      script.src = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";
      script.async = true;
      document.head.append(script);
    }
    render();
    const timeout = window.setTimeout(() => { if (!window.turnstile) scriptError(); }, 15_000);
    return () => {
      disposed = true;
      clearTimeout(timeout);
      script.removeEventListener("load", render);
      script.removeEventListener("error", scriptError);
      if (widget !== undefined) window.turnstile?.remove(widget);
    };
  }, [siteKey, resetKey, compact, retry, onToken]);

  return <>
    <div ref={container} className={interactive ? "mt-3" : undefined} aria-label="Browser verification" />
    {failed && <p className="mt-3 text-[.9rem] leading-[1.5] text-content-muted" role="alert">
      Verification couldn’t load. <button className="cursor-pointer underline focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4" type="button" onClick={() => setRetry((value) => value + 1)}>Try again</button>
    </p>}
  </>;
}
