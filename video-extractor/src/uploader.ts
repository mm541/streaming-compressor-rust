// ════════════════════════════════════════════════
// Video Uploader — Non-Blocking Main Thread API
// ════════════════════════════════════════════════
//
// Wraps the extract-worker in a clean promise-based API.
// ALL heavy work (demux, decode, encode, pack, upload)
// runs in a background Web Worker. The main thread just
// receives progress updates.
//
// Usage:
//   import { createVideoUploader } from 'video-extractor';
//
//   const uploader = createVideoUploader();
//   await uploader.extractAndUpload(file, {
//     uploadUrl: '/api/upload',
//     onProgress: (p) => updateUI(p),
//   });

import type { ExtractionProgress, VideoMetadata } from './types';

export interface UploaderCallbacks {
  /** Upload URL base */
  uploadUrl?: string;
  /** Upload session ID */
  uploadId?: string;
  /** Number of parallel upload fragments */
  parallelism?: number;
  /** Extractor options override */
  extractorOptions?: Record<string, any>;
  /** Extraction progress (demux, decode, encode) */
  onProgress?: (progress: ExtractionProgress) => void;
  /** Each frame as it's extracted */
  onFrame?: (blob: Blob, index: number, timestampMs: number) => void;
  /** Upload fragment progress */
  onUploadProgress?: (info: { fragmentIndex: number; total: number; percent: number }) => void;
  /** Final completion with full manifest */
  onComplete?: (manifest: any) => void;
  /** Error handler */
  onError?: (error: Error) => void;
}

export interface VideoUploader {
  /** Extract audio + frames and upload — fully non-blocking */
  extractAndUpload(file: File, options?: UploaderCallbacks): Promise<any>;
  /** Extract only (no upload) — fully non-blocking */
  extractOnly(file: File, options?: Partial<UploaderCallbacks>): Promise<{
    audio: Blob;
    frames: { blob: Blob; timestampMs: number }[];
    metadata: VideoMetadata;
  }>;
  /** Terminate the worker */
  destroy(): void;
}

/**
 * Creates a video uploader that runs entirely in a Web Worker.
 *
 * @example
 * ```ts
 * const uploader = createVideoUploader();
 *
 * await uploader.extractAndUpload(videoFile, {
 *   uploadUrl: '/api/upload',
 *   onProgress: (p) => {
 *     progressBar.style.width = `${p.percent}%`;
 *     statusText.textContent = p.message;
 *   },
 *   onUploadProgress: ({ percent }) => {
 *     uploadBar.style.width = `${percent}%`;
 *   },
 *   onComplete: (manifest) => {
 *     console.log('Done!', manifest.videoMetadata);
 *   },
 * });
 * ```
 */
export function createVideoUploader(): VideoUploader {
  const worker = new Worker(
    new URL('./extract-worker.ts', import.meta.url),
    { type: 'module' },
  );

  function run(messageType: string, file: File, options: any = {}): Promise<any> {
    return new Promise((resolve, reject) => {
      const {
        onProgress,
        onFrame,
        onUploadProgress,
        onComplete,
        onError,
        ...workerOptions
      } = options;

      worker.onmessage = (e: MessageEvent) => {
        const msg = e.data;

        switch (msg.type) {
          case 'progress':
            onProgress?.(msg.progress);
            break;

          case 'frame':
            onFrame?.(msg.blob, msg.index, msg.timestampMs);
            break;

          case 'upload-progress':
            onUploadProgress?.({
              fragmentIndex: msg.fragmentIndex,
              total: msg.total,
              percent: msg.percent,
            });
            break;

          case 'complete':
            onComplete?.(msg.manifest ?? msg);
            resolve(msg.manifest ?? msg);
            break;

          case 'error':
            const err = new Error(msg.message);
            onError?.(err);
            reject(err);
            break;
        }
      };

      worker.onerror = (e) => {
        const err = new Error(e.message);
        onError?.(err);
        reject(err);
      };

      worker.postMessage({ type: messageType, file, options: workerOptions });
    });
  }

  return {
    extractAndUpload(file, options = {}) {
      return run('extract-and-upload', file, options);
    },

    extractOnly(file, options = {}) {
      return run('extract', file, options);
    },

    destroy() {
      worker.terminate();
    },
  };
}
