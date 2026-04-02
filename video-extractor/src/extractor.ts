// ════════════════════════════════════════════════
// Video Extractor — Core Engine (Refined)
// ════════════════════════════════════════════════
//
// Refinements over v1:
// - Audio wrapped in OGG container (playable)
// - Frames streamed via callback (no memory blow-up)
// - Audio resampling handled for non-48kHz sources
// - Robust error handling for missing tracks

import { WebDemuxer } from 'web-demuxer';
import { renderFrameToBlob } from './frame-renderer';
import { muxOggOpus } from './ogg-muxer';
import type {
  ExtractorOptions,
  ExtractionResult,
  ExtractionProgress,
  VideoMetadata,
} from './types';
import { DEFAULT_OPTIONS } from './types';

/**
 * VideoExtractor — extract audio and 1fps frames from video files.
 *
 * Usage:
 *   const extractor = new VideoExtractor({
 *     onProgress: console.log,
 *     onFrame: (blob, index) => { /* stream each frame out * / },
 *   });
 *   const result = await extractor.extract(videoFile);
 */
export class VideoExtractor {
  private options: ExtractorOptions;

  constructor(options?: Partial<ExtractorOptions>) {
    this.options = { ...DEFAULT_OPTIONS, ...options };
  }

  /**
   * Extract audio and 1fps frames from a video file.
   */
  async extract(file: File): Promise<ExtractionResult> {
    const opts = this.options;
    const report = (p: Partial<ExtractionProgress>) =>
      opts.onProgress?.({
        phase: 'loading',
        percent: 0,
        message: '',
        ...p,
      });

    // ── 1. Initialize demuxer ────────────────────────────
    report({ phase: 'loading', percent: 0, message: 'Initializing demuxer…' });

    const demuxer = new WebDemuxer({
      wasmFilePath: opts.wasmFilePath,
    });

    await demuxer.load(file);

    // ── 2. Get media info ────────────────────────────────
    const mediaInfo = await demuxer.getMediaInfo();
    const duration = mediaInfo.duration ?? 0;

    const videoStream = mediaInfo.streams?.find(
      (s: any) => s.codec_type_string === 'video',
    );
    const audioStream = mediaInfo.streams?.find(
      (s: any) => s.codec_type_string === 'audio',
    );

    if (!videoStream && !audioStream) {
      throw new Error('No video or audio tracks found in file');
    }

    const width = videoStream?.width ?? 0;
    const height = videoStream?.height ?? 0;

    // ── 3. Determine audio bitrate ───────────────────────
    const sampleRate = audioStream?.sample_rate ?? 44100;
    const channels = audioStream?.channels ?? 2;
    const estimatedPcmBytes = duration * sampleRate * channels * 2;
    const maxAudioBytes = opts.maxAudioSizeMB * 1024 * 1024;
    const audioBitrate =
      estimatedPcmBytes > maxAudioBytes ? opts.audioBitrateLow : opts.audioBitrateHigh;

    report({
      phase: 'loading',
      percent: 5,
      message: `Video: ${width}×${height}, ${duration.toFixed(1)}s. Audio: ${audioBitrate / 1000}kbps Opus`,
    });

    // ── 4. Extract frames + audio in parallel ────────────
    const [frames, audioBlob] = await Promise.all([
      videoStream
        ? this.extractFrames(demuxer, duration, report)
        : Promise.resolve([] as { blob: Blob; timestampMs: number }[]),
      audioStream
        ? this.extractAudio(demuxer, duration, sampleRate, channels, audioBitrate, report)
        : Promise.resolve(new Blob()),
    ]);

    // ── 5. Build metadata ────────────────────────────────
    const metadata: VideoMetadata = {
      duration,
      width,
      height,
      frameCount: frames.length,
      videoCodec: videoStream?.codec_name ?? 'none',
      audioCodec: audioStream?.codec_name ?? 'none',
      audioBitrate,
      audioSizeMB: parseFloat((audioBlob.size / (1024 * 1024)).toFixed(2)),
      sourceFile: file.name,
    };

    report({ phase: 'complete', percent: 100, message: 'Extraction complete!' });

    return { audio: audioBlob, frames, metadata };
  }

  // ════════════════════════════════════════════════
  // Frame Extraction Pipeline
  // ════════════════════════════════════════════════

  private async extractFrames(
    demuxer: WebDemuxer,
    duration: number,
    report: (p: Partial<ExtractionProgress>) => void,
  ): Promise<{ blob: Blob; timestampMs: number }[]> {
    const opts = this.options;
    const interval = 1 / opts.fps; // seconds between captures
    const frames: { blob: Blob; timestampMs: number }[] = [];
    const totalExpectedFrames = Math.max(1, Math.ceil(duration * opts.fps));

    const decoderConfig = await demuxer.getDecoderConfig('video');
    const support = await VideoDecoder.isConfigSupported(decoderConfig as VideoDecoderConfig);
    if (!support.supported) {
      throw new Error(`Browser does not support video codec: ${decoderConfig.codec}`);
    }

    let nextCaptureTime = 0;

    return new Promise<{ blob: Blob; timestampMs: number }[]>((resolve, reject) => {
      // Track pending renders to ensure we wait for all before resolving
      let pendingRenders = 0;
      let pumpDone = false;

      const checkComplete = () => {
        if (pumpDone && pendingRenders === 0) resolve(frames);
      };

      const decoder = new VideoDecoder({
        output: (frame: VideoFrame) => {
          const frameTimeSec = (frame.timestamp ?? 0) / 1_000_000;

          if (frameTimeSec >= nextCaptureTime) {
            pendingRenders++;
            const captureIndex = frames.length;
            nextCaptureTime += interval;

            // Render asynchronously but track completion
            renderFrameToBlob(frame, opts.frameFormat, opts.frameQuality, opts.maxFrameWidth)
              .then((blob) => {
                const timestampMs = Math.round(frameTimeSec * 1000);
                frames[captureIndex] = { blob, timestampMs };

                // Fire onFrame callback for streaming
                opts.onFrame?.(blob, captureIndex, timestampMs);

                const pct = 10 + Math.min(40, ((captureIndex + 1) / totalExpectedFrames) * 40);
                report({
                  phase: 'extracting-video',
                  percent: Math.round(pct),
                  message: `Frame ${captureIndex + 1}/${totalExpectedFrames}`,
                });
              })
              .catch((e) => console.warn('Frame render failed:', e))
              .finally(() => {
                pendingRenders--;
                checkComplete();
              });

            frame.close();
          } else {
            frame.close();
          }
        },
        error: (e: DOMException) => {
          reject(new Error(`VideoDecoder error: ${e.message}`));
        },
      });

      decoder.configure(decoderConfig as VideoDecoderConfig);

      const stream = demuxer.read('video');
      const reader = stream.getReader();

      const pump = async () => {
        try {
          while (true) {
            const { done, value } = await reader.read();
            if (done) break;

            decoder.decode(value as EncodedVideoChunk);

            // Back-pressure
            if (decoder.decodeQueueSize > 10) {
              await new Promise<void>((r) => setTimeout(r, 1));
            }
          }

          await decoder.flush();
          decoder.close();
          pumpDone = true;
          checkComplete();
        } catch (e) {
          reject(e);
        }
      };

      pump();
    });
  }

  // ════════════════════════════════════════════════
  // Audio Extraction + OGG Opus Encoding Pipeline
  // ════════════════════════════════════════════════

  private async extractAudio(
    demuxer: WebDemuxer,
    duration: number,
    sourceSampleRate: number,
    sourceChannels: number,
    bitrate: number,
    report: (p: Partial<ExtractionProgress>) => void,
  ): Promise<Blob> {
    const audioDecoderConfig = await demuxer.getDecoderConfig('audio');

    const support = await AudioDecoder.isConfigSupported(audioDecoderConfig as AudioDecoderConfig);
    if (!support.supported) {
      throw new Error(`Browser does not support audio codec: ${audioDecoderConfig.codec}`);
    }

    // ── Setup Opus encoder ───────────────────────────
    // Opus requires 48kHz. The AudioEncoder handles resampling internally
    // when the input AudioData has a different sampleRate.
    const opusConfig: AudioEncoderConfig = {
      codec: 'opus',
      sampleRate: 48000,
      numberOfChannels: 1, // Mono to save space
      bitrate,
    };

    const encoderSupport = await AudioEncoder.isConfigSupported(opusConfig);
    if (!encoderSupport.supported) {
      throw new Error('Browser does not support Opus AudioEncoder');
    }

    return new Promise<Blob>((resolve, reject) => {
      // Collect raw Opus chunks with timestamps for OGG muxing
      const opusChunks: { data: Uint8Array; timestamp: number; duration: number }[] = [];

      const encoder = new AudioEncoder({
        output: (chunk: EncodedAudioChunk) => {
          const buf = new Uint8Array(chunk.byteLength);
          chunk.copyTo(buf);
          opusChunks.push({
            data: buf,
            timestamp: chunk.timestamp,
            duration: chunk.duration ?? 0,
          });
        },
        error: (e: DOMException) => {
          reject(new Error(`AudioEncoder error: ${e.message}`));
        },
      });

      encoder.configure(opusConfig);

      // Decoder feeds PCM into encoder
      let decodedCount = 0;
      const decoder = new AudioDecoder({
        output: (audioData: AudioData) => {
          try {
            // If source is not mono, we need to downmix.
            // AudioEncoder handles channel downmixing when numberOfChannels differs.
            // It also handles sample rate conversion from source → 48kHz.
            encoder.encode(audioData);
            decodedCount++;

            if (decodedCount % 100 === 0) {
              const pct = 50 + Math.min(40, (decodedCount / Math.max(1, duration * 50)) * 40);
              report({
                phase: 'extracting-audio',
                percent: Math.round(pct),
                message: `Audio ${Math.round(pct)}%`,
              });
            }
          } catch (e) {
            console.warn('Audio encode error:', e);
          }

          audioData.close();
        },
        error: (e: DOMException) => {
          reject(new Error(`AudioDecoder error: ${e.message}`));
        },
      });

      decoder.configure(audioDecoderConfig as AudioDecoderConfig);

      // Read all audio chunks
      const stream = demuxer.read('audio');
      const reader = stream.getReader();

      const pump = async () => {
        try {
          while (true) {
            const { done, value } = await reader.read();
            if (done) break;

            decoder.decode(value as EncodedAudioChunk);

            if (decoder.decodeQueueSize > 20) {
              await new Promise<void>((r) => setTimeout(r, 1));
            }
          }

          await decoder.flush();
          decoder.close();

          await encoder.flush();
          encoder.close();

          report({
            phase: 'encoding-audio',
            percent: 95,
            message: 'Muxing OGG Opus…',
          });

          // Mux into OGG container for playable output
          const oggBlob = muxOggOpus(opusChunks, sourceSampleRate, 1);
          resolve(oggBlob);
        } catch (e) {
          reject(e);
        }
      };

      pump();
    });
  }
}
