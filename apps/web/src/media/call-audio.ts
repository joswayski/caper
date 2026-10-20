import { DpdfnetPreparation } from "./dpdfnet-preparation.ts";
import type { AudioSetup, NoiseSuppression } from "./microphone.ts";
import { NoiseAssets } from "./noise-assets.ts";
import { DEFAULT_VOICE_PROCESSING_STRENGTH } from "./voice-processing.ts";

/** The call's microphone choices and model preparation, which a local recording
 * (a voice message) borrows: it sounds like the call, and the noise model the
 * call already downloaded and prepared is reused instead of prepared again. */
export interface CallAudio {
  noiseSuppression: NoiseSuppression;
  audioSetup: AudioSetup;
  voiceProcessingStrength: number;
  /** A call is joining or holds its own processed capture. */
  live: boolean;
  assets: NoiseAssets;
  dpdfnet: DpdfnetPreparation;
  /** Prepares a spare model for the next capture (the call's, when idle). */
  prepare(): void;
}

let current: (() => CallAudio) | undefined;
let standalone: CallAudio | undefined;

/** The newest call client: the one the page shows. */
export function provideCallAudio(read: () => CallAudio) {
  current = read;
}

/** Without a call client (a page with no voice), the call defaults and a page-wide preparation. */
export function callAudio(): CallAudio {
  if (current) return current();
  standalone ??= {
    noiseSuppression: "dpdfnet8",
    audioSetup: "headphones",
    voiceProcessingStrength: DEFAULT_VOICE_PROCESSING_STRENGTH,
    live: false,
    assets: new NoiseAssets(),
    dpdfnet: new DpdfnetPreparation(),
    prepare() {
      void this.dpdfnet.prepare().catch(() => undefined);
    },
  };
  return standalone;
}
