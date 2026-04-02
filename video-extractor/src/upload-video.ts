// ════════════════════════════════════════════════
// Upload Video — Fragment-Based Parallel Upload
// ════════════════════════════════════════════════
//
// Instead of N HTTP requests per file (7200+ for long videos),
// this packs all extracted data into K fragments (K = thread count)
// and uploads them in parallel — exactly like the core Rust publisher.
//
// Flow:
//   1. Extract audio + frames → blobs
//   2. Concatenate all blobs into one byte stream
//   3. Build a manifest with byte offsets (like core StreamEntry)
//   4. Split into K fragments (K ≈ navigator.hardwareConcurrency)
//   5. Upload K fragments in parallel
//   6. Send manifest so backend knows where each file starts/ends

import { VideoExtractor } from './extractor';
import type { ExtractorOptions, ExtractionProgress, VideoMetadata } from './types';

// ── Types ────────────────────────────────────────

export interface FileEntry {
  /** Relative path in the virtual directory */
  path: string;
  /** Byte offset in the concatenated stream */
  byteOffset: number;
  /** Size in bytes */
  size: number;
  /** MIME type */
  mimeType: string;
}

export interface UploadManifest {
  /** Total concatenated size */
  totalSize: number;
  /** Number of fragments uploaded */
  fragmentCount: number;
  /** Fragment size in bytes */
  fragmentSize: number;
  /** Original video filename */
  sourceFile: string;
  /** Video metadata (duration, codec, etc.) */
  videoMetadata: VideoMetadata;
  /** File entries with byte offsets */
  entries: FileEntry[];
}

export interface UploadVideoOptions {
  /** Partial extractor options */
  extractorOptions?: Partial<ExtractorOptions>;
  /** Upload URL base (default: /api/upload) */
  uploadUrl?: string;
  /** Upload ID / session identifier */
  uploadId?: string;
  /** Number of parallel fragments (default: navigator.hardwareConcurrency or 4) */
  parallelism?: number;
  /** Called during extraction phases */
  onExtractionProgress?: (progress: ExtractionProgress) => void;
  /** Called during upload phases */
  onUploadProgress?: (info: { fragmentIndex: number; total: number; percent: number }) => void;
  /** Called when everything is done */
  onComplete?: (manifest: UploadManifest) => void;
  /** Called on error */
  onError?: (error: Error) => void;
}

// ── Main API ─────────────────────────────────────

/**
 * Upload a video by extracting its contents and sending them in K parallel fragments.
 *
 * @param videoFile - The video file to process
 * @param opts - Upload options
 * @returns The upload manifest
 */
export async function uploadVideo(
  videoFile: File,
  opts: UploadVideoOptions = {},
): Promise<UploadManifest> {
  const {
    extractorOptions,
    uploadUrl = '/api/upload',
    uploadId = crypto.randomUUID(),
    parallelism = (typeof navigator !== 'undefined' ? navigator.hardwareConcurrency : 4) || 4,
    onExtractionProgress,
    onUploadProgress,
    onComplete,
    onError,
  } = opts;

  try {
    // ── 1. Extract ──────────────────────────────
    const extractor = new VideoExtractor({
      ...extractorOptions,
      onProgress: onExtractionProgress,
    });

    const result = await extractor.extract(videoFile);
    const { audio, frames, metadata } = result;
    const baseName = videoFile.name.replace(/\.[^.]+$/, '');

    // ── 2. Build file entries & collect blobs ───
    const blobs: Blob[] = [];
    const entries: FileEntry[] = [];
    let offset = 0;

    // Audio
    if (audio.size > 0) {
      entries.push({
        path: `${baseName}/audio.ogg`,
        byteOffset: offset,
        size: audio.size,
        mimeType: 'audio/ogg',
      });
      blobs.push(audio);
      offset += audio.size;
    }

    // Frames
    for (let i = 0; i < frames.length; i++) {
      const mime = frames[i].blob.type || 'image/webp';
      const ext = mime === 'image/jpeg' ? 'jpg' : mime.split('/')[1] || 'webp';
      const name = `frame_${String(i + 1).padStart(4, '0')}_ts${frames[i].timestampMs}ms.${ext}`;
      entries.push({
        path: `${baseName}/frames/${name}`,
        byteOffset: offset,
        size: frames[i].blob.size,
        mimeType: mime,
      });
      blobs.push(frames[i].blob);
      offset += frames[i].blob.size;
    }

    // Metadata JSON
    const metaBlob = new Blob([JSON.stringify(metadata, null, 2)], {
      type: 'application/json',
    });
    entries.push({
      path: `${baseName}/metadata.json`,
      byteOffset: offset,
      size: metaBlob.size,
      mimeType: 'application/json',
    });
    blobs.push(metaBlob);
    offset += metaBlob.size;

    const totalSize = offset;

    // ── 3. Concatenate into single ArrayBuffer ──
    const concatenated = new Uint8Array(totalSize);
    let writePos = 0;

    for (const blob of blobs) {
      const buf = new Uint8Array(await blob.arrayBuffer());
      concatenated.set(buf, writePos);
      writePos += buf.byteLength;
    }

    // ── 4. Split into K fragments ───────────────
    const fragmentCount = Math.min(parallelism, Math.ceil(totalSize / (64 * 1024))); // at least 64KB per fragment
    const fragmentSize = Math.ceil(totalSize / fragmentCount);

    const manifest: UploadManifest = {
      totalSize,
      fragmentCount,
      fragmentSize,
      sourceFile: videoFile.name,
      videoMetadata: metadata,
      entries,
    };

    // ── 5. Upload fragments in parallel ─────────
    const uploadPromises: Promise<void>[] = [];

    for (let i = 0; i < fragmentCount; i++) {
      const start = i * fragmentSize;
      const end = Math.min(start + fragmentSize, totalSize);
      const fragmentData = concatenated.slice(start, end);

      const promise = uploadFragment(
        uploadUrl,
        uploadId,
        i,
        fragmentCount,
        fragmentData,
      ).then(() => {
        onUploadProgress?.({
          fragmentIndex: i,
          total: fragmentCount,
          percent: Math.round(((i + 1) / fragmentCount) * 100),
        });
      });

      uploadPromises.push(promise);
    }

    await Promise.all(uploadPromises);

    // ── 6. Send manifest ────────────────────────
    await uploadManifest(uploadUrl, uploadId, manifest);

    onComplete?.(manifest);
    return manifest;
  } catch (e: any) {
    const error = e instanceof Error ? e : new Error(String(e));
    onError?.(error);
    throw error;
  }
}

// ── HTTP Helpers ─────────────────────────────────

const MAX_RETRIES = 3;
const BASE_DELAY_MS = 500;

async function uploadFragment(
  baseUrl: string,
  uploadId: string,
  index: number,
  total: number,
  data: Uint8Array,
): Promise<void> {
  const url = `${baseUrl}/fragment?uploadId=${encodeURIComponent(uploadId)}&index=${index}&total=${total}`;
  const headers: Record<string, string> = {
    'Content-Type': 'application/octet-stream',
  };

  for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
    try {
      const response = await fetch(url, { method: 'POST', headers, body: new Blob([data as BlobPart]) });
      if (response.ok) return;
      if (response.status < 500) throw new Error(`Upload failed: ${response.status}`);
    } catch (error) {
      if (attempt === MAX_RETRIES) throw error;
    }
    const delay = BASE_DELAY_MS * Math.pow(2, attempt);
    await new Promise((resolve) => setTimeout(resolve, delay));
  }
}

async function uploadManifest(
  baseUrl: string,
  uploadId: string,
  manifest: UploadManifest,
): Promise<void> {
  const url = `${baseUrl}/finalize?uploadId=${encodeURIComponent(uploadId)}`;
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
  };

  const response = await fetch(url, {
    method: 'POST',
    headers,
    body: JSON.stringify(manifest),
  });

  if (!response.ok) {
    throw new Error(`Manifest upload failed: ${response.status}`);
  }
}
