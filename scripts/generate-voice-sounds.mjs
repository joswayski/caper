// Writes the voice chimes every client plays (shared/sounds/voice-join.wav and
// voice-leave.wav): two soft sine notes, rising for a join and falling for a
// leave, quiet enough to sit under a conversation. Use --check to fail on drift.
import assert from "node:assert/strict";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const directory = fileURLToPath(new URL("../shared/sounds/", import.meta.url));
const checkOnly = process.argv.includes("--check");
const RATE = 48_000;
const LENGTH = 0.42;
const PEAK = 0.3;

/** Notes as [frequency Hz, start s]; each rings out with a short attack and an exponential decay. */
function chime(notes) {
  const samples = new Float64Array(Math.round(RATE * LENGTH));
  for (const [frequency, start] of notes) {
    for (let i = Math.round(start * RATE); i < samples.length; i++) {
      const t = i / RATE - start;
      const envelope = Math.min(1, t / 0.012) * Math.exp(-t / 0.09);
      const phase = 2 * Math.PI * frequency * t;
      samples[i] += envelope * (Math.sin(phase) + 0.12 * Math.sin(2 * phase));
    }
  }
  // Fade the tail so the file ends in silence, then scale to the peak.
  const fade = Math.round(RATE * 0.04);
  for (let i = 0; i < fade; i++) samples[samples.length - 1 - i] *= i / fade;
  const loudest = samples.reduce((max, value) => Math.max(max, Math.abs(value)), 0);
  return samples.map((value) => (value / loudest) * PEAK);
}

function wav(samples) {
  const data = samples.length * 2;
  const file = Buffer.alloc(44 + data);
  file.write("RIFF", 0, "ascii");
  file.writeUInt32LE(36 + data, 4);
  file.write("WAVEfmt ", 8, "ascii");
  file.writeUInt32LE(16, 16);
  file.writeUInt16LE(1, 20); // PCM
  file.writeUInt16LE(1, 22); // mono
  file.writeUInt32LE(RATE, 24);
  file.writeUInt32LE(RATE * 2, 28);
  file.writeUInt16LE(2, 32);
  file.writeUInt16LE(16, 34);
  file.write("data", 36, "ascii");
  file.writeUInt32LE(data, 40);
  samples.forEach((value, index) => file.writeInt16LE(Math.round(value * 32_767), 44 + index * 2));
  return file;
}

const sounds = {
  "voice-join.wav": chime([
    [659.26, 0], // E5
    [987.77, 0.085], // B5
  ]),
  "voice-leave.wav": chime([
    [987.77, 0], // B5
    [659.26, 0.085], // E5
  ]),
};

mkdirSync(directory, { recursive: true });
for (const [name, samples] of Object.entries(sounds)) {
  const bytes = wav(samples);
  if (checkOnly) assert.ok(bytes.equals(readFileSync(directory + name)), `${name} is out of date`);
  else writeFileSync(directory + name, bytes);
}
