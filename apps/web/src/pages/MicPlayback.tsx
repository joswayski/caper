import { useEffect, useRef, useState } from "react";
import { recordReceivedAudio, type ReceivedRecording } from "../media/recording";

export default function MicPlayback({ stream }: { stream: MediaStream }) {
  const liveRef = useRef<HTMLAudioElement>(null);
  const playbackRef = useRef<HTMLAudioElement>(null);
  const urlRef = useRef<string | undefined>(undefined);
  const sessionRef = useRef<ReceivedRecording | undefined>(undefined);
  const generation = useRef(0);
  const [live, setLive] = useState(false);
  const [recording, setRecording] = useState(false);
  const [url, setUrl] = useState<string>();
  const [error, setError] = useState<string>();

  const teardown = () => {
    generation.current++;
    sessionRef.current?.stop();
    sessionRef.current = undefined;
    playbackRef.current?.pause();
    if (urlRef.current) URL.revokeObjectURL(urlRef.current);
    urlRef.current = undefined;
    setRecording(false);
    setUrl(undefined);
  };

  useEffect(() => {
    setLive(false);
    setError(undefined);
    return () => teardown();
  }, [stream]);
  useEffect(() => {
    const element = liveRef.current;
    if (!element) return;
    element.srcObject = live ? stream : null;
    if (live) void element.play().catch(() => setError("Live playback was blocked. Use the audio play control."));
    return () => { element.pause(); element.srcObject = null; };
  }, [live, stream]);

  const record = () => {
    teardown();
    setLive(false);
    setError(undefined);
    const current = generation.current;
    try {
      const session = recordReceivedAudio(stream);
      sessionRef.current = session;
      setRecording(true);
      void session.result.then((blob) => {
        if (generation.current !== current) return;
        sessionRef.current = undefined;
        setRecording(false);
        urlRef.current = URL.createObjectURL(blob);
        setUrl(urlRef.current);
      }).catch((reason) => {
        if (generation.current !== current) return;
        sessionRef.current = undefined;
        setRecording(false);
        setError(reason instanceof Error ? reason.message : "Audio recording failed.");
      });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Audio recording failed.");
    }
  };
  const toggleLive = () => {
    teardown();
    setError(undefined);
    setLive((current) => !current);
  };

  return <div>
    {!live && <p className="noise-status">Speak for five seconds, then press play to hear the received audio. The snippet stays only in this tab and is cleared when you stop the test.</p>}
    <div className="control-buttons">
      {!live && !recording && <button type="button" onClick={record}>{url ? "Record again" : "Record 5 seconds"}</button>}
      <button type="button" className={live ? "active" : ""} aria-pressed={live} onClick={toggleLive}>{live ? "Back to snippet" : "Listen live"}</button>
    </div>
    {recording && <p className="noise-status" role="status">Recording received audio… 5 seconds.</p>}
    {error && <p className="noise-status" role="alert">{error}</p>}
    {live && <><p className="noise-status" role="status">Listening live to received audio.</p><audio ref={liveRef} controls style={{ maxWidth: "100%", width: "100%" }} /></>}
    {!live && url && <><p className="noise-status" role="status">Ready. Press play to listen to your five-second snippet.</p><audio ref={playbackRef} aria-label="Recorded microphone test" controls src={url} style={{ maxWidth: "100%", width: "100%" }} /></>}
  </div>;
}
