// HDR → SDR transcode: decoded 10-bit frames go through `HdrToneMapper` and
// are encoded as BT.709 I420. Mediabunny's Conversion resizes and rotates on a
// 2D canvas before any `process` hook, which would hand us browser-converted
// 8-bit frames, so this drives the sinks and sources directly. Rotation stays
// track metadata; frames keep their coded orientation.
import type * as Mediabunny from "mediabunny";
import { HdrToneMapper, supportedHdrFrameFormat, type HdrTransfer } from "./hdr.ts";

export interface HdrTranscodeOptions {
  /** Output display size (after rotation), even. */
  width: number;
  height: number;
  transfer: HdrTransfer;
  codec: Mediabunny.VideoCodec;
  bitrate: number;
  /** "copy" remuxes the audio packets; undefined means no audio track. */
  audioCodec?: "copy" | Mediabunny.AudioCodec;
  audioBitrate: number;
  progress: (fraction: number) => void;
  signal?: AbortSignal;
}

const BT709 = { primaries: "bt709", transfer: "bt709", matrix: "bt709", fullRange: false } as const;

/** MP4 bytes, or undefined when this browser cannot read the frames (the
 * caller then keeps the original). Throws on cancellation or encoder errors. */
export async function transcodeHdrVideo(media: typeof Mediabunny, input: Mediabunny.Input, options: HdrTranscodeOptions): Promise<ArrayBuffer | undefined> {
  const track = await input.getPrimaryVideoTrack();
  if (!track) return;
  const audio = options.audioCodec ? await input.getPrimaryAudioTrack() : null;
  const rotation = await track.getRotation();
  const sideways = rotation === 90 || rotation === 270;
  const [codedWidth, codedHeight] = sideways ? [options.height, options.width] : [options.width, options.height];
  const duration = await input.computeDuration().catch(() => 0);

  const sink = new media.VideoSampleSink(track);
  const first = await sink.getSample(await track.getFirstTimestamp());
  if (!first) return;
  try {
    if (!supportedHdrFrameFormat(first.format)) return;
  } finally {
    first.close();
  }
  let mapper: HdrToneMapper;
  try {
    mapper = new HdrToneMapper(codedWidth, codedHeight, options.transfer);
  } catch {
    return;
  }

  const output = new media.Output({ format: new media.Mp4OutputFormat({ fastStart: "in-memory" }), target: new media.BufferTarget() });
  const video = new media.VideoSampleSource({ codec: options.codec, bitrate: options.bitrate });
  output.addVideoTrack(video, { rotation });
  let audioPump: (() => Promise<void>) | undefined;
  if (audio && options.audioCodec === "copy") {
    const codec = await audio.getCodec();
    const decoderConfig = await audio.getDecoderConfig();
    if (!codec || !decoderConfig) { mapper.dispose(); return; }
    const source = new media.EncodedAudioPacketSource(codec);
    output.addAudioTrack(source);
    audioPump = async () => {
      let meta: EncodedAudioChunkMetadata | undefined = { decoderConfig };
      for await (const packet of new media.EncodedPacketSink(audio).packets()) {
        if (options.signal?.aborted) break;
        await source.add(packet, meta);
        meta = undefined;
      }
      source.close();
    };
  } else if (audio && options.audioCodec && options.audioCodec !== "copy") {
    const source = new media.AudioSampleSource({ codec: options.audioCodec, bitrate: options.audioBitrate });
    output.addAudioTrack(source);
    audioPump = async () => {
      for await (const sample of new media.AudioSampleSink(audio).samples()) {
        try {
          if (options.signal?.aborted) break;
          await source.add(sample);
        } finally {
          sample.close();
        }
      }
      source.close();
    };
  }
  await output.start();
  const videoPump = async () => {
    for await (const sample of sink.samples()) {
      try {
        if (options.signal?.aborted) break;
        const frame = await mapper.map(sample);
        const mapped = new media.VideoSample(frame.data, {
          format: "I420", codedWidth: frame.width, codedHeight: frame.height, layout: frame.layout,
          timestamp: sample.timestamp, duration: sample.duration, colorSpace: BT709,
        });
        try {
          await video.add(mapped);
        } finally {
          mapped.close();
        }
        if (duration > 0) options.progress(Math.min(1, (sample.timestamp + sample.duration) / duration));
      } finally {
        sample.close();
      }
    }
    video.close();
  };
  try {
    await Promise.all([videoPump(), audioPump?.()]);
    if (options.signal?.aborted) {
      await output.cancel();
      return;
    }
    await output.finalize();
    return output.target.buffer ?? undefined;
  } catch (error) {
    await output.cancel().catch(() => undefined);
    throw error;
  } finally {
    mapper.dispose();
  }
}
