export const DEFAULT_VOICE_PROCESSING_STRENGTH = 25;

export interface VoiceProcessingNodes {
  highpass: BiquadFilterNode;
  warmth: BiquadFilterNode;
  presence: BiquadFilterNode;
  compressor: DynamicsCompressorNode;
  makeup: GainNode;
  limiter: DynamicsCompressorNode;
}

export function clampVoiceProcessingStrength(strength: number) {
  return Math.max(0, Math.min(strength, 100));
}

function setParameter(parameter: AudioParam, value: number, currentTime: number, smooth: boolean) {
  if (smooth) {
    parameter.cancelScheduledValues(currentTime);
    parameter.setTargetAtTime(value, currentTime, 0.015);
  } else {
    parameter.value = value;
  }
}

export function setVoiceProcessingStrength(
  nodes: VoiceProcessingNodes,
  strength: number,
  currentTime: number,
  smooth = false,
) {
  const amount = clampVoiceProcessingStrength(strength) / 100;
  setParameter(nodes.highpass.frequency, 75 * amount, currentTime, smooth);
  setParameter(nodes.warmth.gain, 2 * amount, currentTime, smooth);
  setParameter(nodes.presence.gain, 1.5 * amount, currentTime, smooth);
  setParameter(nodes.compressor.ratio, 1 + 2 * amount, currentTime, smooth);
  setParameter(nodes.makeup.gain, 1.35 ** amount, currentTime, smooth);
}

export function createVoiceProcessingNodes(context: BaseAudioContext, strength: number): VoiceProcessingNodes {
  const highpass = context.createBiquadFilter();
  highpass.type = "highpass";
  highpass.Q.value = 0.7;
  const warmth = context.createBiquadFilter();
  warmth.type = "lowshelf";
  warmth.frequency.value = 180;
  const presence = context.createBiquadFilter();
  presence.type = "peaking";
  presence.frequency.value = 3_000;
  presence.Q.value = 0.8;
  const compressor = context.createDynamicsCompressor();
  compressor.threshold.value = -24;
  compressor.knee.value = 12;
  compressor.attack.value = 0.008;
  compressor.release.value = 0.18;
  const makeup = context.createGain();
  const limiter = context.createDynamicsCompressor();
  limiter.threshold.value = -2;
  limiter.knee.value = 0;
  limiter.ratio.value = 20;
  limiter.attack.value = 0.002;
  limiter.release.value = 0.08;
  const nodes = { highpass, warmth, presence, compressor, makeup, limiter };
  setVoiceProcessingStrength(nodes, strength, context.currentTime);
  return nodes;
}

export function connectVoiceProcessing(
  source: AudioNode,
  destination: AudioNode,
  nodes: VoiceProcessingNodes,
) {
  source.connect(nodes.highpass)
    .connect(nodes.warmth)
    .connect(nodes.presence)
    .connect(nodes.compressor)
    .connect(nodes.makeup)
    .connect(nodes.limiter)
    .connect(destination);
}

export function disconnectVoiceProcessing(nodes: VoiceProcessingNodes) {
  nodes.highpass.disconnect();
  nodes.warmth.disconnect();
  nodes.presence.disconnect();
  nodes.compressor.disconnect();
  nodes.makeup.disconnect();
  nodes.limiter.disconnect();
}
