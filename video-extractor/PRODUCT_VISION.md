# The Serverless AI Video Architecture

### Core Concept
Instead of uploading heavy `.mp4` video files to a server, paying for video hosting, and wasting CPU cycles doing server-side processing, **the browser does all the heavy lifting using native WebCodecs API before the upload even begins.**

### Event A: The Upload Pipeline (Client-Side)
1. **Client-Side Demux & Decode:** User selects a video. The browser uses WebAssembly to crack open the video container (MP4, MKV, WebM).
2. **Frame Extraction (1 FPS):** The browser extracts exactly 1 frame per second, downscales each frame to `768px`, and compresses it to `40%` quality WebP.
3. **Audio Extraction:** The browser extracts the PCM audio, resamples it to 48kHz, and encodes it to highly-compressed Opus inside an `.ogg` container.
4. **Timestamping:** Every extracted WebP filename is baked with its exact millisecond timestamp (e.g., `frame_0001_ts1000ms.webp`). 
5. **Parallel Fragment Upload:** All frames and audio are concatenated in memory into a single binary stream. The browser splits this stream into K fragments (e.g., 4) and uploads them in parallel, followed by a JSON manifest.

### Event B: 100% Lazy Asynchronous Backend
The backend **must not** block the user while processing. The flow is completely asynchronous:
1. **Immediate Response:** When the Spring Boot backend receives the Finalize Manifest request and reassembles the `audio.ogg` and frames to a temporary drive, it instantly returns `HTTP 202 Accepted` to the frontend. This explicitly tells the client *"I have accepted the files and put them in a queue, but processing is not finished"*. The user's screen says *"Processing your video..."* and they can navigate away.
2. **RabbitMQ / Background Queue:** The backend publishes a task to a message broker (e.g., RabbitMQ or Celery).
3. **S3 Image Storage (Background):** A background worker uploads all `768px` WebP frames directly to AWS S3 (or Cloudflare R2). 
4. **Audio Transcription (Background):** The worker passes the `audio.ogg` file to an AI (like Whisper) to generate a text transcript with millisecond timestamps (e.g., `[start_ms: 10000, end_ms: 15000, text: "Hello world."]`).
5. **PostgreSQL Indexing:** The worker saves *only* the transcript text and S3 pointers to the database:
   - Table `video_frames`: `[video_id, timestamp_ms, s3_image_url]`
   - Table `video_transcripts`: `[video_id, start_ms, end_ms, text]`
6. **Trash the Output:** The `.ogg` file and the unpacked frames are deleted from the disk.

### Event C: The Next-Gen UI & AI Slicing
Because of this architecture, the frontend UI does not need to stream an `.mp4` video.
1. **Text-First Playback:** The UI streams the audio file. As the audio plays, the UI reads the `audio.currentTime` and instantly renders the exact `<img>` from S3.
2. **Interactive Transcript:** The UI renders the transcript text like a document. If the user clicks a sentence, the audio jumps to that timestamp, and the image updates.
3. **AI Scene Slicing:** To "select a part of the video", the user highlights text in the transcript or uses a dual-handle slider. 
4. **Zero-Cost Generation:** If the user selects a 15-second range, the frontend sends those timestamps to the backend. The backend queries PostgreSQL to find the frames in that millisecond range, grabs those 15 lightweight images from S3, and passes them to a Vision AI (GPT-4o or Gemini 1.5) to generate whatever content is needed.

### Summary of Benefits
* **Cost:** Zero actual video hosting. Object storage for 20KB images is effectively free.
* **UX:** Uploads finish fast, and background workers process the data lazily without freezing the client.
* **Speed:** The user's browser is utilized for edge computing.
* **AI Ready:** The frames are pre-optimized (768px, 40% WebP) specifically so they won't blow up token limits when passed to Vision Models. 
* **Database Health:** PostgreSQL stays lean and lightning-fast because it only stores text and URLs, never blobs.
