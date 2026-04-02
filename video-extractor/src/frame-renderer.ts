// ════════════════════════════════════════════════
// Frame Renderer — VideoFrame → JPEG/PNG Blob
// ════════════════════════════════════════════════

/**
 * Renders a VideoFrame to a JPEG or PNG blob using OffscreenCanvas.
 * Works in both main thread and Web Workers.
 *
 * If `maxWidth` is provided and the frame is wider, it's downscaled
 * proportionally before export (saves bandwidth for AI-only frames).
 */
export async function renderFrameToBlob(
  frame: VideoFrame,
  format: 'image/jpeg' | 'image/png' | 'image/webp' = 'image/webp',
  quality: number = 0.4,
  maxWidth: number = 768,
): Promise<Blob> {
  let width = frame.displayWidth;
  let height = frame.displayHeight;

  // Downscale if wider than maxWidth
  if (maxWidth > 0 && width > maxWidth) {
    const scale = maxWidth / width;
    width = maxWidth;
    height = Math.round(height * scale);
  }

  const canvas = new OffscreenCanvas(width, height);
  const ctx = canvas.getContext('2d')!;

  // Draw the VideoFrame onto the canvas (resized if needed)
  ctx.drawImage(frame, 0, 0, width, height);

  // Export as blob
  const blob = await canvas.convertToBlob({
    type: format,
    quality: format === 'image/jpeg' || format === 'image/webp' ? quality : undefined,
  });

  return blob;
}
