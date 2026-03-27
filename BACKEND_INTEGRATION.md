# Backend Integration Guide

This document describes exactly what your **Spring Boot Gateway** and **Python Processing Service** need to implement to accept, reconstruct, and process video files uploaded by the WASM streaming compressor.

---

## Architecture Overview

```
Browser (WASM + Web Workers)
    │
    │  N parallel HTTP POSTs (1MB compressed chunks)
    ▼
Spring Boot Gateway
    │
    ├──► Saves compressed chunks to shared volume: /shared/uploads/<fileId>/
    │
    │  Publishes lightweight metadata to RabbitMQ (NOT binary data)
    │  Queue: "chunk.metadata" (filename, index, checksum — ~200 bytes)
    │  Queue: "upload.finalize" (filename, totalChunks)
    ▼
RabbitMQ Broker
    │
    ▼
Python Consumer
    │
    ├──► Reads chunks from shared volume
    ├──► Decompresses LZ4 → assembles full file
    ├──► Extracts audio (ffmpeg)
    ├──► Extracts video frames (OpenCV / ffmpeg)
    └──► Stores processed output
```

---

## 1. Spring Boot Gateway

Spring Boot receives chunks over HTTP and saves them to a **shared volume** that Python can also access. It sends only tiny metadata messages through RabbitMQ.

### 1.1 Maven Dependencies

```xml
<dependency>
    <groupId>org.springframework.boot</groupId>
    <artifactId>spring-boot-starter-amqp</artifactId>
</dependency>
```

### 1.2 Chunk Receiver Endpoint

```java
@RestController
@RequestMapping("/api")
public class ChunkUploadController {

    @Autowired
    private RabbitTemplate rabbitTemplate;

    @Value("${upload.shared-volume}")
    private String sharedVolume;

    @PostMapping("/upload-chunk")
    public ResponseEntity<String> uploadChunk(
            @RequestParam("file") String filename,
            @RequestParam("index") int chunkIndex,
            @RequestParam("checksum") String checksum,
            @RequestHeader("X-Original-Content-Type") String contentType,
            @RequestHeader("X-Compression-Skipped") String compressionSkipped,
            @RequestHeader(value = "X-Relative-Path", required = false) String relativePath,
            @RequestBody byte[] chunkData
    ) throws IOException {
        // Use relativePath for directory uploads, filename for single files
        String filePath = (relativePath != null) 
            ? java.net.URLDecoder.decode(relativePath, "UTF-8") 
            : filename;
        String fileId = filePath.replaceAll("[^a-zA-Z0-9._/-]", "_");

        // Save chunk to shared volume preserving directory structure
        Path chunkDir = Path.of(sharedVolume, fileId);
        Files.createDirectories(chunkDir);
        Files.write(chunkDir.resolve("chunk_" + chunkIndex), chunkData);

        return ResponseEntity.ok("OK");
    }
}
```

### 1.3 Finalize Endpoint

Handles both single-file and directory uploads:

```java
// Single file finalize (query params)
@PostMapping("/finalize")
public ResponseEntity<String> finalizeUpload(
        @RequestBody(required = false) Map<String, Object> body,
        @RequestParam(value = "file", required = false) String filename,
        @RequestParam(value = "totalChunks", required = false) Integer totalChunks,
        @RequestParam(value = "compressionSkipped", defaultValue = "false") boolean compressionSkipped
) {
    // Directory upload: body contains a manifest
    if (body != null && "directory".equals(body.get("type"))) {
        rabbitTemplate.convertAndSend(
            "compressor.exchange", "upload.finalize", body
        );
        return ResponseEntity.ok("Directory finalization started");
    }

    // Single file upload
    String fileId = filename.replaceAll("[^a-zA-Z0-9._-]", "_");
    Map<String, Object> payload = Map.of(
        "type", "single",
        "fileId", fileId,
        "filename", filename,
        "totalChunks", totalChunks,
        "compressionSkipped", compressionSkipped
    );
    rabbitTemplate.convertAndSend(
        "compressor.exchange", "upload.finalize", payload
    );
    return ResponseEntity.ok("Finalization started");
}
```

### 1.4 RabbitMQ Queue Configuration

```java
@Configuration
public class RabbitConfig {

    @Bean
    public DirectExchange compressorExchange() {
        return new DirectExchange("compressor.exchange");
    }

    // Metadata queue: lightweight chunk arrival notifications
    @Bean
    public Queue metadataQueue() {
        return QueueBuilder.durable("chunk.metadata").build();
    }

    // Finalize queue: triggers Python assembly + processing
    @Bean
    public Queue finalizeQueue() {
        return QueueBuilder.durable("upload.finalize").build();
    }

    @Bean
    public Binding metadataBinding(Queue metadataQueue, DirectExchange compressorExchange) {
        return BindingBuilder.bind(metadataQueue)
            .to(compressorExchange).with("chunk.metadata");
    }

    @Bean
    public Binding finalizeBinding(Queue finalizeQueue, DirectExchange compressorExchange) {
        return BindingBuilder.bind(finalizeQueue)
            .to(compressorExchange).with("upload.finalize");
    }
}
```

### 1.5 Application Configuration

```yaml
# application.yml
spring:
  servlet:
    multipart:
      max-file-size: 10MB
      max-request-size: 10MB
  rabbitmq:
    host: localhost
    port: 5672
    username: guest
    password: guest

upload:
  shared-volume: /shared/uploads

server:
  max-http-request-header-size: 16KB
```

### 1.6 CORS Configuration

```java
@Configuration
public class WebConfig implements WebMvcConfigurer {
    @Override
    public void addCorsMappings(CorsRegistry registry) {
        registry.addMapping("/api/**")
            .allowedOrigins("http://localhost:3000")
            .allowedMethods("POST")
            .allowedHeaders("*")
            .exposedHeaders("X-Original-Content-Type",
                          "X-Chunk-Checksum",
                          "X-Compression-Skipped");
    }
}
```

---

## 2. Python Processing Service

Python reads compressed chunks from the shared volume, decompresses them, assembles the full file, and runs video processing.

### 2.1 Install Dependencies

```bash
pip install lz4 blake3 pika
# For video processing:
pip install opencv-python  # or: apt install python3-opencv
# ffmpeg must be installed system-wide:
apt install ffmpeg
```

### 2.2 Core Decompression Functions

```python
import struct
import lz4.block
import blake3

def decompress_chunk(compressed_bytes: bytes) -> bytes:
    """Decompress a single LZ4 chunk from the WASM compressor.
    Format: [4-byte LE u32 uncompressed_size][LZ4 block payload]
    """
    uncompressed_size = struct.unpack('<I', compressed_bytes[:4])[0]
    payload = compressed_bytes[4:]
    return lz4.block.decompress(payload, uncompressed_size=uncompressed_size)

def verify_checksum(data: bytes, expected_checksum: str) -> bool:
    """Verify blake3 checksum matches what WASM computed."""
    actual = blake3.blake3(data).hexdigest()
    return actual == expected_checksum
```

### 2.3 File Assembly

```python
import os

SHARED_VOLUME = "/shared/uploads"
OUTPUT_DIR = "/data/processed"

def assemble_file(file_id: str, filename: str, total_chunks: int, 
                  compression_skipped: bool = False) -> str:
    """
    Read all compressed chunks from the shared volume,
    decompress them in order, and write the complete output file.
    Returns the path to the assembled file.
    """
    output_path = os.path.join(OUTPUT_DIR, filename)
    os.makedirs(os.path.dirname(output_path), exist_ok=True)
    chunk_dir = os.path.join(SHARED_VOLUME, file_id)

    with open(output_path, 'wb') as output:
        for i in range(total_chunks):
            chunk_path = os.path.join(chunk_dir, f"chunk_{i}")
            chunk_data = open(chunk_path, 'rb').read()

            if compression_skipped:
                output.write(chunk_data)
            else:
                decompressed = decompress_chunk(chunk_data)
                output.write(decompressed)

    # Clean up chunk directory after assembly
    for f in os.listdir(chunk_dir):
        os.remove(os.path.join(chunk_dir, f))
    os.rmdir(chunk_dir)

    print(f"Assembled: {output_path} ({total_chunks} chunks)")
    return output_path
```

### 2.4 Video Processing Pipeline

Once the file is fully assembled, process it:

```python
import subprocess
import cv2

def extract_audio(video_path: str, output_dir: str) -> str:
    """Extract audio track from video using ffmpeg."""
    basename = os.path.splitext(os.path.basename(video_path))[0]
    audio_path = os.path.join(output_dir, f"{basename}.wav")

    subprocess.run([
        "ffmpeg", "-i", video_path,
        "-vn",              # No video
        "-acodec", "pcm_s16le",  # WAV format
        "-ar", "16000",     # 16kHz sample rate (good for speech models)
        "-ac", "1",         # Mono
        audio_path
    ], check=True)

    print(f"Audio extracted: {audio_path}")
    return audio_path

def extract_frames(video_path: str, output_dir: str, fps: int = 1) -> list:
    """Extract video frames at specified FPS using OpenCV."""
    basename = os.path.splitext(os.path.basename(video_path))[0]
    frames_dir = os.path.join(output_dir, f"{basename}_frames")
    os.makedirs(frames_dir, exist_ok=True)

    cap = cv2.VideoCapture(video_path)
    video_fps = cap.get(cv2.CAP_PROP_FPS)
    frame_interval = int(video_fps / fps)  # Extract 1 frame per second

    frame_paths = []
    frame_count = 0
    saved_count = 0

    while cap.isOpened():
        ret, frame = cap.read()
        if not ret:
            break

        if frame_count % frame_interval == 0:
            frame_path = os.path.join(frames_dir, f"frame_{saved_count:06d}.jpg")
            cv2.imwrite(frame_path, frame)
            frame_paths.append(frame_path)
            saved_count += 1

        frame_count += 1

    cap.release()
    print(f"Extracted {saved_count} frames to {frames_dir}")
    return frame_paths

def process_video(video_path: str) -> dict:
    """Full video processing pipeline."""
    output_dir = os.path.dirname(video_path)

    audio_path = extract_audio(video_path, output_dir)
    frame_paths = extract_frames(video_path, output_dir, fps=1)

    return {
        "video": video_path,
        "audio": audio_path,
        "frames": frame_paths,
        "frame_count": len(frame_paths)
    }
```

### 2.5 RabbitMQ Consumer

```python
import pika
import json

def on_finalize_message(ch, method, properties, body):
    """
    Called when Spring Boot publishes a finalize command.
    Handles both single-file and directory uploads.
    """
    payload = json.loads(body)
    upload_type = payload.get('type', 'single')

    try:
        if upload_type == 'directory':
            # Directory upload: manifest contains list of files
            manifest = payload['manifest']
            total_files = payload['totalFiles']
            print(f"Directory finalize: {total_files} files")

            for entry in manifest:
                rel_path = entry['relativePath']
                total_chunks = entry['totalChunks']
                file_id = rel_path.replace('/', '_').replace(' ', '_')

                # Check extension to know if chunks are compressed
                ext = rel_path.rsplit('.', 1)[-1].lower()
                skip = ext in SKIP_EXTENSIONS
                
                output_path = os.path.join(OUTPUT_DIR, rel_path)
                os.makedirs(os.path.dirname(output_path), exist_ok=True)
                assemble_file(file_id, output_path, total_chunks, compression_skipped=skip)

            print(f"Directory assembled: {total_files} files under {OUTPUT_DIR}")

        else:
            # Single file upload
            file_id = payload['fileId']
            filename = payload['filename']
            total_chunks = payload['totalChunks']
            compression_skipped = payload.get('compressionSkipped', False)

            print(f"Single file finalize: {filename} ({total_chunks} chunks)")
            output_path = os.path.join(OUTPUT_DIR, filename)
            assemble_file(file_id, output_path, total_chunks, compression_skipped)

            # Run video processing if it's a video file
            ext = filename.rsplit('.', 1)[-1].lower()
            if ext in ('mp4', 'webm', 'mkv', 'avi', 'mov'):
                result = process_video(output_path)
                print(f"Video processed: {result['frame_count']} frames")

        ch.basic_ack(delivery_tag=method.delivery_tag)

    except Exception as e:
        print(f"ERROR: {e}")
        ch.basic_nack(delivery_tag=method.delivery_tag, requeue=False)

# Extensions that were uploaded without compression
SKIP_EXTENSIONS = {
    'mp4', 'webm', 'mkv', 'avi', 'mov', 'wmv', 'flv',
    'mp3', 'aac', 'ogg', 'flac', 'wma', 'opus',
    'jpg', 'jpeg', 'png', 'gif', 'webp', 'avif', 'heic',
    'zip', 'gz', 'bz2', 'xz', 'zst', 'rar', '7z',
    'pdf', 'docx', 'xlsx', 'pptx',
}

def start_consumer():
    connection = pika.BlockingConnection(
        pika.ConnectionParameters(host='localhost')
    )
    channel = connection.channel()
    channel.basic_qos(prefetch_count=1)

    channel.basic_consume(
        queue='upload.finalize', 
        on_message_callback=on_finalize_message
    )

    print("Python processor started. Waiting for uploads...")
    channel.start_consuming()

if __name__ == '__main__':
    start_consumer()
```

---

## 3. Frontend → Finalize Call

After all chunks upload, the frontend triggers assembly:

```javascript
onComplete: async (result) => {
    await fetch(
        `/api/finalize?file=${file.name}&totalChunks=${result.totalChunks}&compressionSkipped=${result.skipped}`, 
        { method: 'POST' }
    );
}
```

---

## 4. Data Format Quick Reference

| Field | Format | Example |
|-------|--------|---------|
| Compressed chunk | `[4-byte LE u32 size][LZ4 block]` | `\x00\x10\x00\x00...` |
| Checksum | blake3 hex string (64 chars) | `a7f3b2c1d4e5...` |
| Chunk index | 0-based integer | `0, 1, 2, ...` |
| Chunk size | Configurable (default 1MB) | `1048576` bytes |
| Skip header | `X-Compression-Skipped: true/false` | `true` |
| Content type | `X-Original-Content-Type` | `video/mp4` |

---

## 5. Shared Volume Layout

```
/shared/uploads/
    └── video_mp4/                  ← fileId (sanitized filename)
        ├── chunk_0                 ← 1MB compressed LZ4 chunk
        ├── chunk_1
        ├── chunk_2
        └── ...

/data/processed/
    ├── video.mp4                   ← fully assembled original video
    ├── video.wav                   ← extracted audio (16kHz mono WAV)
    └── video_frames/
        ├── frame_000000.jpg        ← extracted frames at 1 FPS
        ├── frame_000001.jpg
        └── ...
```

---

## 6. RabbitMQ Queue Topology

```
compressor.exchange (DirectExchange)
    │
    ├── routing_key: "chunk.metadata"  →  chunk.metadata (Queue)
    │                                      └── Optional: track upload progress
    │
    └── routing_key: "upload.finalize" →  upload.finalize (Queue)
                                           └── Python consumer: assemble + process
```

**Why only metadata through RabbitMQ?**
- RabbitMQ is optimized for **small messages** (~KB). Pushing 1MB binary blobs degrades broker performance.
- Chunks are saved to a **shared volume** (NFS, Docker volume, or local disk) which is much faster for large binary data.
- Python reads chunks at **disk speed** instead of deserializing from AMQP.

---

## 7. Docker Compose (Reference)

```yaml
version: '3.8'
services:
  rabbitmq:
    image: rabbitmq:3-management
    ports:
      - "5672:5672"
      - "15672:15672"  # Management UI

  spring-boot:
    build: ./backend-spring
    ports:
      - "8080:8080"
    volumes:
      - shared-uploads:/shared/uploads
    depends_on:
      - rabbitmq

  python-processor:
    build: ./backend-python
    volumes:
      - shared-uploads:/shared/uploads
      - processed-data:/data/processed
    depends_on:
      - rabbitmq

volumes:
  shared-uploads:
  processed-data:
```

---

## 8. Checklist

- [ ] Docker: Start RabbitMQ (`docker run -p 5672:5672 -p 15672:15672 rabbitmq:3-management`)
- [ ] Docker: Create shared volume between Spring Boot and Python
- [ ] Spring Boot: Add `spring-boot-starter-amqp` dependency
- [ ] Spring Boot: Create `/api/upload-chunk` (save to shared volume + publish metadata)
- [ ] Spring Boot: Create `/api/finalize` (verify chunks + publish finalize command)
- [ ] Spring Boot: Configure exchange, queues, and bindings
- [ ] Spring Boot: Configure CORS
- [ ] Python: Install `lz4`, `blake3`, `pika`, `opencv-python`
- [ ] Python: Install `ffmpeg` system package
- [ ] Python: Implement `assemble_file()` — read from shared volume, decompress, write output
- [ ] Python: Implement `extract_audio()` with ffmpeg
- [ ] Python: Implement `extract_frames()` with OpenCV
- [ ] Python: Implement RabbitMQ consumer on `upload.finalize` queue
- [ ] Frontend: Call `/api/finalize` in `onComplete` callback
