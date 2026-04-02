// ════════════════════════════════════════════════
// OGG Opus Muxer — Wraps raw Opus frames in OGG container
// ════════════════════════════════════════════════
//
// Raw Opus EncodedAudioChunk bytes are not playable.
// They must be wrapped in an OGG container with:
//   Page 1: OpusHead header
//   Page 2: OpusTags (comment) header
//   Page 3+: Audio data pages
//
// Reference: RFC 7845 (OGG Encapsulation for Opus)

const OGG_CAPTURE = new Uint8Array([0x4f, 0x67, 0x67, 0x53]); // "OggS"

/** Build a complete OGG Opus file from raw Opus encoded chunks. */
export function muxOggOpus(
  chunks: { data: Uint8Array; timestamp: number; duration: number }[],
  sampleRate: number,
  channels: number,
): Blob {
  const pages: Uint8Array[] = [];
  let granulePosition = 0n;
  let pageSequence = 0;
  const serialNumber = (Math.random() * 0xffffffff) >>> 0;

  // ── Page 1: OpusHead ───────────────────────────
  const opusHead = createOpusHead(channels, sampleRate);
  pages.push(
    createOggPage(opusHead, serialNumber, pageSequence++, 0n, 0x02 /* BOS */),
  );

  // ── Page 2: OpusTags ───────────────────────────
  const opusTags = createOpusTags();
  pages.push(
    createOggPage(opusTags, serialNumber, pageSequence++, 0n, 0x00),
  );

  // ── Page 3+: Audio data ────────────────────────
  // Group chunks into pages (max ~255 segments per page)
  const MAX_SEGMENTS_PER_PAGE = 200;
  let pageChunks: Uint8Array[] = [];
  let pageDurationSamples = 0n;

  for (let i = 0; i < chunks.length; i++) {
    const chunk = chunks[i];
    pageChunks.push(chunk.data);

    // Duration in samples at 48kHz (Opus always uses 48kHz internally)
    const durationSamples = BigInt(Math.round((chunk.duration / 1_000_000) * 48000));
    pageDurationSamples += durationSamples;

    const isLast = i === chunks.length - 1;
    const pageFull = pageChunks.length >= MAX_SEGMENTS_PER_PAGE;

    if (pageFull || isLast) {
      granulePosition += pageDurationSamples;
      const headerType = isLast ? 0x04 /* EOS */ : 0x00;

      pages.push(
        createOggPageMultiPacket(
          pageChunks,
          serialNumber,
          pageSequence++,
          granulePosition,
          headerType,
        ),
      );

      pageChunks = [];
      pageDurationSamples = 0n;
    }
  }

  return new Blob(pages as BlobPart[], { type: 'audio/ogg; codecs=opus' });
}

// ── OpusHead packet (RFC 7845, Section 5.1) ──────

function createOpusHead(channels: number, inputSampleRate: number): Uint8Array {
  const buf = new ArrayBuffer(19);
  const view = new DataView(buf);
  const bytes = new Uint8Array(buf);

  // "OpusHead"
  bytes.set([0x4f, 0x70, 0x75, 0x73, 0x48, 0x65, 0x61, 0x64], 0);
  view.setUint8(8, 1); // Version
  view.setUint8(9, channels); // Channel count
  view.setUint16(10, 3840, true); // Pre-skip (80ms at 48kHz)
  view.setUint32(12, inputSampleRate, true); // Input sample rate (informational)
  view.setInt16(16, 0, true); // Output gain
  view.setUint8(18, 0); // Channel mapping family (0 = mono/stereo)

  return bytes;
}

// ── OpusTags packet (RFC 7845, Section 5.2) ──────

function createOpusTags(): Uint8Array {
  const vendor = 'video-extractor';
  const vendorBytes = new TextEncoder().encode(vendor);

  const buf = new ArrayBuffer(8 + 4 + vendorBytes.length + 4);
  const view = new DataView(buf);
  const bytes = new Uint8Array(buf);

  // "OpusTags"
  bytes.set([0x4f, 0x70, 0x75, 0x73, 0x54, 0x61, 0x67, 0x73], 0);
  view.setUint32(8, vendorBytes.length, true); // Vendor string length
  bytes.set(vendorBytes, 12); // Vendor string
  view.setUint32(12 + vendorBytes.length, 0, true); // Comment count = 0

  return bytes;
}

// ── OGG Page construction ────────────────────────

function createOggPage(
  packet: Uint8Array,
  serialNumber: number,
  pageSequence: number,
  granulePosition: bigint,
  headerType: number,
): Uint8Array {
  return createOggPageMultiPacket([packet], serialNumber, pageSequence, granulePosition, headerType);
}

function createOggPageMultiPacket(
  packets: Uint8Array[],
  serialNumber: number,
  pageSequence: number,
  granulePosition: bigint,
  headerType: number,
): Uint8Array {
  // Build segment table
  const segmentTable: number[] = [];
  for (const packet of packets) {
    let remaining = packet.byteLength;
    while (remaining >= 255) {
      segmentTable.push(255);
      remaining -= 255;
    }
    segmentTable.push(remaining); // terminating segment (0-254)
  }

  const totalDataSize = packets.reduce((sum, p) => sum + p.byteLength, 0);
  const headerSize = 27 + segmentTable.length;
  const pageSize = headerSize + totalDataSize;
  const page = new Uint8Array(pageSize);
  const view = new DataView(page.buffer);

  // Header
  page.set(OGG_CAPTURE, 0); // "OggS"
  view.setUint8(4, 0); // Version
  view.setUint8(5, headerType); // Header type
  view.setBigUint64(6, granulePosition, true); // Granule position
  view.setUint32(14, serialNumber, true); // Serial number
  view.setUint32(18, pageSequence, true); // Page sequence
  view.setUint32(22, 0, true); // CRC (will compute below)
  view.setUint8(26, segmentTable.length); // Number of segments

  // Segment table
  for (let i = 0; i < segmentTable.length; i++) {
    page[27 + i] = segmentTable[i];
  }

  // Packet data
  let offset = headerSize;
  for (const packet of packets) {
    page.set(packet, offset);
    offset += packet.byteLength;
  }

  // CRC-32 (OGG uses a specific polynomial)
  const crc = oggCrc32(page);
  view.setUint32(22, crc, true);

  return page;
}

// ── OGG CRC-32 (polynomial 0x04C11DB7) ──────────

const CRC_TABLE = buildCrcTable();

function buildCrcTable(): Uint32Array {
  const table = new Uint32Array(256);
  for (let i = 0; i < 256; i++) {
    let r = i << 24;
    for (let j = 0; j < 8; j++) {
      r = (r & 0x80000000) ? ((r << 1) ^ 0x04c11db7) : (r << 1);
    }
    table[i] = r >>> 0;
  }
  return table;
}

function oggCrc32(data: Uint8Array): number {
  let crc = 0;
  for (let i = 0; i < data.length; i++) {
    crc = ((crc << 8) ^ CRC_TABLE[((crc >>> 24) & 0xff) ^ data[i]]) >>> 0;
  }
  return crc;
}
