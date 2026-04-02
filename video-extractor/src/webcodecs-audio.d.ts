// ════════════════════════════════════════════════
// WebCodecs Audio API Type Declarations
// ════════════════════════════════════════════════
//
// TypeScript's DOM lib doesn't include AudioEncoder/AudioDecoder types yet.
// These are the minimal declarations needed for our usage.

interface AudioDecoderInit {
  output: (data: AudioData) => void;
  error: (error: DOMException) => void;
}

interface AudioDecoderConfig {
  codec: string;
  sampleRate?: number;
  numberOfChannels?: number;
  description?: BufferSource;
}

interface AudioDecoderSupport {
  supported: boolean;
  config: AudioDecoderConfig;
}

declare class AudioDecoder {
  constructor(init: AudioDecoderInit);
  static isConfigSupported(config: AudioDecoderConfig): Promise<AudioDecoderSupport>;
  configure(config: AudioDecoderConfig): void;
  decode(chunk: EncodedAudioChunk): void;
  flush(): Promise<void>;
  close(): void;
  readonly state: string;
  readonly decodeQueueSize: number;
}

interface AudioEncoderInit {
  output: (chunk: EncodedAudioChunk, metadata?: EncodedAudioChunkMetadata) => void;
  error: (error: DOMException) => void;
}

interface AudioEncoderConfig {
  codec: string;
  sampleRate: number;
  numberOfChannels: number;
  bitrate?: number;
  opus?: {
    application?: string;
    complexity?: number;
    signal?: string;
    usedtx?: boolean;
  };
}

interface AudioEncoderSupport {
  supported: boolean;
  config: AudioEncoderConfig;
}

interface EncodedAudioChunkMetadata {
  decoderConfig?: AudioDecoderConfig;
}

declare class AudioEncoder {
  constructor(init: AudioEncoderInit);
  static isConfigSupported(config: AudioEncoderConfig): Promise<AudioEncoderSupport>;
  configure(config: AudioEncoderConfig): void;
  encode(data: AudioData): void;
  flush(): Promise<void>;
  close(): void;
  readonly state: string;
  readonly encodeQueueSize: number;
}

interface EncodedAudioChunkInit {
  type: string;
  timestamp: number;
  duration?: number;
  data: BufferSource;
}

declare class EncodedAudioChunk {
  constructor(init: EncodedAudioChunkInit);
  readonly type: string;
  readonly timestamp: number;
  readonly duration: number | null;
  readonly byteLength: number;
  copyTo(destination: BufferSource): void;
}

declare class AudioData {
  readonly format: string | null;
  readonly sampleRate: number;
  readonly numberOfFrames: number;
  readonly numberOfChannels: number;
  readonly duration: number;
  readonly timestamp: number;
  close(): void;
  clone(): AudioData;
  copyTo(destination: BufferSource, options: { planeIndex: number }): void;
}
