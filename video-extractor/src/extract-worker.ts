// ════════════════════════════════════════════════
// Video Extract + Upload Worker
// ════════════════════════════════════════════════
// Runs the ENTIRE pipeline off the main thread:
//   1. Demux video (web-demuxer WASM)
//   2. Decode video → 1fps JPEG frames
//   3. Decode audio → Opus OGG
//   4. Pack into byte stream
//   5. Upload K fragments in parallel
//   6. Send manifest
//
// Message Protocol:
//   Main → Worker:
//     { type: 'extract-and-upload', file: File, options: {...} }
//
//   Worker → Main:
//     { type: 'progress', progress: ExtractionProgress }
//     { type: 'frame', blob: Blob, index: number }
//     { type: 'upload-progress', fragmentIndex, total, percent }
//     { type: 'complete', manifest: UploadManifest }
//     { type: 'error', message: string }

import { VideoExtractor } from './extractor';
import type { VideoMetadata, ExtractionProgress } from './types';

interface FileEntry {
  path: string;
  byteOffset: number;
  size: number;
  mimeType: string;
}

interface UploadManifest {
  totalSize: number;
  fragmentCount: number;
  fragmentSize: number;
  sourceFile: string;
  videoMetadata: VideoMetadata;
  entries: FileEntry[];
}

const MAX_RETRIES = 3;
const BASE_DELAY_MS = 500;

self.onmessage = async (e: MessageEvent) => {
  const { type, file, options = {} } = e.data;

  if (type === 'extract') {
    // Extract only — no upload
    await handleExtractOnly(file, options);
  } else if (type === 'extract-and-upload') {
    // Full pipeline — extract + upload
    await handleExtractAndUpload(file, options);
  }
};

// ── Extract Only ─────────────────────────────────

async function handleExtractOnly(file: File, options: any) {
  try {
    const extractor = new VideoExtractor({
      ...options,
      onProgress: (progress: ExtractionProgress) => {
        self.postMessage({ type: 'progress', progress });
      },
      onFrame: (blob: Blob, index: number) => {
        self.postMessage({ type: 'frame', blob, index });
      },
    });

    const result = await extractor.extract(file);
    self.postMessage({
      type: 'complete',
      audio: result.audio,
      frames: result.frames,
      metadata: result.metadata,
    });
  } catch (err: any) {
    self.postMessage({ type: 'error', message: err?.message ?? String(err) });
  }
}

// ── Extract + Upload ─────────────────────────────

async function handleExtractAndUpload(file: File, options: any) {
  const {
    uploadUrl = '/api/upload',
    uploadId = crypto.randomUUID(),
    parallelism = 4,
    extractorOptions = {},
  } = options;

  try {
    // ── 1. Extract ──────────────────────────
    self.postMessage({
      type: 'progress',
      progress: { phase: 'loading', percent: 0, message: 'Starting extraction…' },
    });

    const extractor = new VideoExtractor({
      ...extractorOptions,
      onProgress: (progress: ExtractionProgress) => {
        self.postMessage({ type: 'progress', progress });
      },
      onFrame: (blob: Blob, index: number) => {
        self.postMessage({ type: 'frame', blob, index });
      },
    });

    const { audio, frames, metadata } = await extractor.extract(file);
    const baseName = file.name.replace(/\.[^.]+$/, '');

    // ── 2. Build entries + concatenate ───────
    const blobs: Blob[] = [];
    const entries: FileEntry[] = [];
    let offset = 0;

    if (audio.size > 0) {
      entries.push({ path: `${baseName}/audio.ogg`, byteOffset: offset, size: audio.size, mimeType: 'audio/ogg' });
      blobs.push(audio);
      offset += audio.size;
    }

    for (let i = 0; i < frames.length; i++) {
      const mime = frames[i].blob.type || 'image/webp';
      const ext = mime === 'image/jpeg' ? 'jpg' : mime.split('/')[1] || 'webp';
      const name = `frame_${String(i + 1).padStart(4, '0')}_ts${frames[i].timestampMs}ms.${ext}`;
      entries.push({ path: `${baseName}/frames/${name}`, byteOffset: offset, size: frames[i].blob.size, mimeType: mime });
      blobs.push(frames[i].blob);
      offset += frames[i].blob.size;
    }

    const metaBlob = new Blob([JSON.stringify(metadata, null, 2)], { type: 'application/json' });
    entries.push({ path: `${baseName}/metadata.json`, byteOffset: offset, size: metaBlob.size, mimeType: 'application/json' });
    blobs.push(metaBlob);
    offset += metaBlob.size;

    const totalSize = offset;

    // ── 3. Concatenate ──────────────────────
    self.postMessage({
      type: 'progress',
      progress: { phase: 'encoding-audio', percent: 92, message: 'Packing data…' },
    });

    const concatenated = new Uint8Array(totalSize);
    let writePos = 0;
    for (const blob of blobs) {
      const buf = new Uint8Array(await blob.arrayBuffer());
      concatenated.set(buf, writePos);
      writePos += buf.byteLength;
    }

    // ── 4. Split into K fragments ───────────
    const fragmentCount = Math.min(parallelism, Math.ceil(totalSize / (64 * 1024)));
    const fragmentSize = Math.ceil(totalSize / fragmentCount);

    const manifest: UploadManifest = {
      totalSize,
      fragmentCount,
      fragmentSize,
      sourceFile: file.name,
      videoMetadata: metadata,
      entries,
    };

    // ── 5. Upload fragments in parallel ─────
    let completedFragments = 0;
    const uploadPromises: Promise<void>[] = [];

    for (let i = 0; i < fragmentCount; i++) {
      const start = i * fragmentSize;
      const end = Math.min(start + fragmentSize, totalSize);
      const fragmentData = concatenated.slice(start, end);

      const promise = uploadFragment(uploadUrl, uploadId, i, fragmentCount, fragmentData).then(() => {
        completedFragments++;
        self.postMessage({
          type: 'upload-progress',
          fragmentIndex: i,
          total: fragmentCount,
          percent: Math.round((completedFragments / fragmentCount) * 100),
        });
      });

      uploadPromises.push(promise);
    }

    await Promise.all(uploadPromises);

    // ── 6. Send manifest ────────────────────
    await uploadManifest(uploadUrl, uploadId, manifest);

    self.postMessage({ type: 'complete', manifest });
  } catch (err: any) {
    self.postMessage({ type: 'error', message: err?.message ?? String(err) });
  }
}

// ── HTTP Helpers ─────────────────────────────────

async function uploadFragment(
  baseUrl: string, uploadId: string, index: number, total: number, data: Uint8Array,
): Promise<void> {
  const url = `${baseUrl}/fragment?uploadId=${encodeURIComponent(uploadId)}&index=${index}&total=${total}`;
  for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
    try {
      const res = await fetch(url, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream' }, body: new Blob([data as BlobPart]) });
      if (res.ok) return;
      if (res.status < 500) throw new Error(`Upload failed: ${res.status}`);
    } catch (e) {
      if (attempt === MAX_RETRIES) throw e;
    }
    await new Promise((r) => setTimeout(r, BASE_DELAY_MS * Math.pow(2, attempt)));
  }
}

async function uploadManifest(baseUrl: string, uploadId: string, manifest: UploadManifest): Promise<void> {
  const res = await fetch(`${baseUrl}/finalize?uploadId=${encodeURIComponent(uploadId)}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(manifest),
  });
  if (!res.ok) throw new Error(`Manifest upload failed: ${res.status}`);
}
