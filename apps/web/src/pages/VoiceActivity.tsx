import { useEffect, useRef } from "react";
import { watchVoiceActivity } from "../media/voice-activity";

interface VoiceActivityProps {
  stream?: MediaStream;
  muted: boolean;
  onActivityChange(active: boolean): void;
}

export default function VoiceActivity({ stream, muted, onActivityChange }: VoiceActivityProps) {
  const activityCallback = useRef(onActivityChange);
  activityCallback.current = onActivityChange;

  useEffect(() => watchVoiceActivity(stream, muted, (active) => activityCallback.current(active)), [muted, stream]);

  return null;
}
