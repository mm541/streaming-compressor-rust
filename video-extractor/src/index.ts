// ════════════════════════════════════════════════
// Video Extractor — Public API
// ════════════════════════════════════════════════

// Core
export { VideoExtractor } from './extractor';
export { renderFrameToBlob } from './frame-renderer';
export { muxOggOpus } from './ogg-muxer';

// Upload
export { uploadVideo } from './upload-video';
export { createVideoUploader } from './uploader';

// Types
export type {
  ExtractorOptions,
  ExtractionResult,
  ExtractionProgress,
  VideoMetadata,
} from './types';
export type { UploaderCallbacks, VideoUploader } from './uploader';
export { DEFAULT_OPTIONS } from './types';
