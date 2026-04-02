// ════════════════════════════════════════════════
// Video Extractor — Type Definitions
// ════════════════════════════════════════════════

/** Configuration options for the video extractor */
export interface ExtractorOptions {
  /** Frame output format. Default: 'image/webp' */
  frameFormat: 'image/jpeg' | 'image/png' | 'image/webp';
  /** JPEG quality (0-1). Default: 0.4 (AI-optimized). Use 0.7+ for human viewing. */
  frameQuality: number;
  /** Max frame width in pixels. Frames wider than this are downscaled. Default: 768 (AI-optimized). */
  maxFrameWidth: number;
  /** Frames per second to capture. Default: 1 */
  fps: number;
  /** Maximum audio file size in MB. Default: 25 */
  maxAudioSizeMB: number;
  /** Audio bitrate for short videos (under size limit). Default: 64000 */
  audioBitrateHigh: number;
  /** Audio bitrate for long videos (over size limit). Default: 24000 */
  audioBitrateLow: number;
  /** Path to web-demuxer WASM file. Default: CDN */
  wasmFilePath: string;
  /** Progress callback */
  onProgress?: (progress: ExtractionProgress) => void;
  /** Called for each extracted frame — use to stream frames out without buffering all in memory */
  onFrame?: (blob: Blob, index: number, timestampMs: number) => void;
}

/** Progress information during extraction */
export interface ExtractionProgress {
  /** Current processing phase */
  phase: 'loading' | 'extracting-video' | 'extracting-audio' | 'encoding-audio' | 'complete';
  /** Overall progress percentage (0-100) */
  percent: number;
  /** Human-readable status message */
  message: string;
}

/** Result of video extraction */
export interface ExtractionResult {
  /** Opus-encoded audio blob */
  audio: Blob;
  /** Array of extracted frames with their timestamps */
  frames: { blob: Blob; timestampMs: number }[];
  /** Video metadata */
  metadata: VideoMetadata;
}

/** Metadata about the processed video */
export interface VideoMetadata {
  /** Video duration in seconds */
  duration: number;
  /** Video width in pixels */
  width: number;
  /** Video height in pixels */
  height: number;
  /** Number of extracted frames */
  frameCount: number;
  /** Original video codec name */
  videoCodec: string;
  /** Original audio codec name */
  audioCodec: string;
  /** Actual audio bitrate used (bps) */
  audioBitrate: number;
  /** Final audio blob size in MB */
  audioSizeMB: number;
  /** Source filename */
  sourceFile: string;
}

/** Default extractor options */
export const DEFAULT_OPTIONS: ExtractorOptions = {
  frameFormat: 'image/webp',
  frameQuality: 0.4,
  maxFrameWidth: 768,
  fps: 1,
  maxAudioSizeMB: 25,
  audioBitrateHigh: 64_000,
  audioBitrateLow: 24_000,
  wasmFilePath: 'https://cdn.jsdelivr.net/npm/web-demuxer@latest/dist/wasm-files/web-demuxer.wasm',
  onProgress: undefined,
  onFrame: undefined,
};
