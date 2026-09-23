import { useEffect, useState } from "react";
import type { PublicCallClient } from "../media/client";

export default function AudioDiagnostics({ client }: { client: PublicCallClient | undefined }) {
  const [snapshot, setSnapshot] = useState(() => client?.getAudioDiagnostics());
  const [copyStatus, setCopyStatus] = useState("");
  useEffect(() => {
    const update = () => setSnapshot(client?.getAudioDiagnostics());
    update();
    const timer = setInterval(update, 1000);
    return () => clearInterval(timer);
  }, [client]);
  const report = JSON.stringify({
    browser: navigator.userAgent,
    logicalProcessors: navigator.hardwareConcurrency,
    ...snapshot,
  }, null, 2);
  return <section className="audio-debug" aria-label="Audio diagnostics">
    <p>Local diagnostics · refresh resets counters. No audio, device identifiers, or credentials are included. Nothing is uploaded.</p>
    {!snapshot?.captures.length && <p role="status">{snapshot?.captureAttempt === "failed" ? `Capture failed: ${snapshot.captureError}` : snapshot?.captureAttempt === "opening" ? "Opening microphone…" : snapshot?.captureAttempt === "opened" ? "Microphone capture has ended." : "No microphone capture started. Open Mic Test or join voice first."}</p>}
    <button type="button" className="voice-button" onClick={() => void navigator.clipboard.writeText(report).then(() => setCopyStatus("Copied diagnostics"), () => setCopyStatus("Copy failed; select the report below."))}>Copy diagnostics</button>
    <span role="status">{copyStatus}</span>
    <pre>{report}</pre>
  </section>;
}
