import { useEffect, useState, type ComponentProps } from "react";

type Picker = typeof import("./ReactionPicker.tsx").default;

let picker: Picker | undefined;
let loading: Promise<Picker> | undefined;

/** Starts (or joins) the picker's download. A failed download can be started again. */
export function loadReactionPicker() {
  return (loading ??= import("./ReactionPicker.tsx").then(
    (module) => (picker = module.default),
    (error: unknown) => {
      loading = undefined;
      throw error;
    },
  ));
}

/**
 * The picker carries the whole emoji catalog, so it loads when first needed or
 * warmed rather than with the conversation. Once loaded it renders at once; its
 * container has a fixed size, so nothing moves when it appears. A download that
 * fails (offline, or a tab older than the last release) leaves the conversation
 * usable and is retried the next time a picker opens.
 */
export default function LazyReactionPicker(props: ComponentProps<Picker>) {
  const [Loaded, setLoaded] = useState(() => picker);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (Loaded) return;
    let current = true;
    loadReactionPicker().then(
      (loaded) => current && setLoaded(() => loaded),
      () => current && setFailed(true),
    );
    return () => {
      current = false;
    };
  }, [Loaded]);
  if (Loaded) return <Loaded {...props} />;
  return failed ? <p role="alert">Emoji could not be loaded. Close this and try again.</p> : null;
}
