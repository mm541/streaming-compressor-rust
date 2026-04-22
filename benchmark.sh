#!/usr/bin/env bash
set -euo pipefail

# ═══════════════════════════════════════════════════════════════════
#  Streaming Compressor vs tar+zstd — Head-to-Head Benchmark
# ═══════════════════════════════════════════════════════════════════

# Provide the target dataset as the first argument.
if [ -z "${1:-}" ]; then
    echo "Usage: $0 <path_to_dataset> [zstd_level]"
    exit 1
fi

INPUT="$1"
LEVEL="${2:-3}"
BENCH_DIR="./_benchmark"
CLI="./target/release/cli"
TIME_CMD="/usr/bin/time"

# Ensure CLI exists
if [ ! -f "$CLI" ]; then
    echo "Error: CLI binary not found. Please run 'cargo build --release' first."
    exit 1
fi

rm -rf "$BENCH_DIR"
mkdir -p "$BENCH_DIR"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  Dataset: $INPUT"
echo "  Zstd Level: $LEVEL  |  CPU Cores: $(nproc)"
echo "  Zstd CLI: $(zstd --version 2>&1 | head -1)"
echo "═══════════════════════════════════════════════════════════════"

TOTAL_BYTES=$(du -sb "$INPUT" | awk '{print $1}')
FILE_COUNT=$(find "$INPUT" -type f | wc -l)
TOTAL_MB=$(echo "scale=2; $TOTAL_BYTES / 1048576" | bc)
TOTAL_GB=$(echo "scale=2; $TOTAL_BYTES / 1073741824" | bc)
echo "  Size: ${TOTAL_GB} GB (${TOTAL_MB} MB) | ${FILE_COUNT} files"
echo "  Free disk: $(df -h . --output=avail | tail -1 | xargs)"
echo "═══════════════════════════════════════════════════════════════"
echo ""

# ─── ROUND 1: tar + zstd ───────────────────────────────────────
echo "━━━ ROUND 1: tar + zstd -${LEVEL} -T0 ━━━"

echo "[1a] Compress..."
sync
$TIME_CMD -v bash -c "tar cf - '$INPUT' 2>/dev/null | zstd -${LEVEL} -T0 -o '$BENCH_DIR/archive.tar.zst'" 2>"$BENCH_DIR/tar_c.log"

TAR_C_SIZE=$(stat -c%s "$BENCH_DIR/archive.tar.zst")
TAR_C_WALL=$(grep "Elapsed (wall clock)" "$BENCH_DIR/tar_c.log" | awk -F': ' '{print $2}')
TAR_C_RAM=$(grep "Maximum resident" "$BENCH_DIR/tar_c.log" | awk '{print $NF}')
TAR_C_RAM_MB=$(echo "scale=2; $TAR_C_RAM / 1024" | bc)
TAR_C_SIZE_MB=$(echo "scale=2; $TAR_C_SIZE / 1048576" | bc)
TAR_C_RATIO=$(echo "scale=2; $TOTAL_BYTES / $TAR_C_SIZE" | bc)
echo "  Done: ${TAR_C_WALL} | ${TAR_C_SIZE_MB} MB | ${TAR_C_RATIO}x | RAM: ${TAR_C_RAM_MB} MB"

echo "[1b] Decompress..."
mkdir -p "$BENCH_DIR/tar_out"
sync
$TIME_CMD -v bash -c "zstd -d '$BENCH_DIR/archive.tar.zst' --stdout | tar xf - -C '$BENCH_DIR/tar_out'" 2>"$BENCH_DIR/tar_d.log"

TAR_D_WALL=$(grep "Elapsed (wall clock)" "$BENCH_DIR/tar_d.log" | awk -F': ' '{print $2}')
TAR_D_RAM=$(grep "Maximum resident" "$BENCH_DIR/tar_d.log" | awk '{print $NF}')
TAR_D_RAM_MB=$(echo "scale=2; $TAR_D_RAM / 1024" | bc)
echo "  Done: ${TAR_D_WALL} | RAM: ${TAR_D_RAM_MB} MB"

echo "[1c] Cleanup..."
rm -rf "$BENCH_DIR/archive.tar.zst" "$BENCH_DIR/tar_out"
echo ""

# ─── ROUND 2: streaming-compressor ─────────────────────────────
echo "━━━ ROUND 2: streaming-compressor -l ${LEVEL} --no-skip ━━━"

echo "[2a] Compress..."
sync
$TIME_CMD -v $CLI compress "$INPUT" "$BENCH_DIR/our_archive" -l $LEVEL --no-skip 2>"$BENCH_DIR/our_c.log"

OUR_C_SIZE=$(find "$BENCH_DIR/our_archive" -type f -name '*.zst' -exec stat -c%s {} + | awk '{s+=$1} END {print s+0}')
OUR_C_WALL=$(grep "Elapsed (wall clock)" "$BENCH_DIR/our_c.log" | awk -F': ' '{print $2}')
OUR_C_RAM=$(grep "Maximum resident" "$BENCH_DIR/our_c.log" | awk '{print $NF}')
OUR_C_RAM_MB=$(echo "scale=2; $OUR_C_RAM / 1024" | bc)
OUR_C_SIZE_MB=$(echo "scale=2; $OUR_C_SIZE / 1048576" | bc)
OUR_C_RATIO=$(echo "scale=2; $TOTAL_BYTES / $OUR_C_SIZE" | bc)
echo "  Done: ${OUR_C_WALL} | ${OUR_C_SIZE_MB} MB | ${OUR_C_RATIO}x | RAM: ${OUR_C_RAM_MB} MB"

echo "[2b] Decompress..."
mkdir -p "$BENCH_DIR/our_out"
sync
$TIME_CMD -v $CLI decompress "$BENCH_DIR/our_archive" "$BENCH_DIR/our_out" 2>"$BENCH_DIR/our_d.log"

OUR_D_WALL=$(grep "Elapsed (wall clock)" "$BENCH_DIR/our_d.log" | awk -F': ' '{print $2}')
OUR_D_RAM=$(grep "Maximum resident" "$BENCH_DIR/our_d.log" | awk '{print $NF}')
OUR_D_RAM_MB=$(echo "scale=2; $OUR_D_RAM / 1024" | bc)
echo "  Done: ${OUR_D_WALL} | RAM: ${OUR_D_RAM_MB} MB"

echo "[2c] Cleanup..."
rm -rf "$BENCH_DIR/our_archive" "$BENCH_DIR/our_out"
echo ""

# ─── RESULTS ────────────────────────────────────────────────────
echo "╔═══════════════════════════════════════════════════════════════╗"
echo "║                   BENCHMARK RESULTS                         ║"
echo "╠═══════════════════════════════════════════════════════════════╣"
printf "║  Dataset: %-47s  ║\n" "${TOTAL_GB} GB | ${FILE_COUNT} files"
printf "║  Config:  %-47s  ║\n" "Zstd level ${LEVEL} | $(nproc) cores"
echo "╠═══════════════════════════════════════════════════════════════╣"
echo "║                                                             ║"
echo "║  ┌──────────────────┬─────────────────┬─────────────────┐   ║"
echo "║  │                  │  tar + zstd      │  ours           │   ║"
echo "║  ├──────────────────┼─────────────────┼─────────────────┤   ║"
printf "║  │ Compress Time    │ %-15s │ %-15s │   ║\n" "$TAR_C_WALL" "$OUR_C_WALL"
printf "║  │ Compress RAM     │ %-15s │ %-15s │   ║\n" "${TAR_C_RAM_MB} MB" "${OUR_C_RAM_MB} MB"
printf "║  │ Archive Size     │ %-15s │ %-15s │   ║\n" "${TAR_C_SIZE_MB} MB" "${OUR_C_SIZE_MB} MB"
printf "║  │ Ratio            │ %-15s │ %-15s │   ║\n" "${TAR_C_RATIO}x" "${OUR_C_RATIO}x"
echo "║  ├──────────────────┼─────────────────┼─────────────────┤   ║"
printf "║  │ Decompress Time  │ %-15s │ %-15s │   ║\n" "$TAR_D_WALL" "$OUR_D_WALL"
printf "║  │ Decompress RAM   │ %-15s │ %-15s │   ║\n" "${TAR_D_RAM_MB} MB" "${OUR_D_RAM_MB} MB"
echo "║  └──────────────────┴─────────────────┴─────────────────┘   ║"
echo "║                                                             ║"
echo "╚═══════════════════════════════════════════════════════════════╝"

cat > "$BENCH_DIR/results.txt" <<EOF
BENCHMARK: streaming-compressor vs tar+zstd
Dataset: ${TOTAL_GB} GB | ${FILE_COUNT} files
Config: Zstd level ${LEVEL} | $(nproc) cores | --no-skip

tar+zstd compress:   ${TAR_C_WALL}  |  ${TAR_C_SIZE_MB} MB  |  ${TAR_C_RATIO}x  |  RAM: ${TAR_C_RAM_MB} MB
tar+zstd decompress: ${TAR_D_WALL}  |  RAM: ${TAR_D_RAM_MB} MB

ours compress:       ${OUR_C_WALL}  |  ${OUR_C_SIZE_MB} MB  |  ${OUR_C_RATIO}x  |  RAM: ${OUR_C_RAM_MB} MB
ours decompress:     ${OUR_D_WALL}  |  RAM: ${OUR_D_RAM_MB} MB
EOF
echo ""
echo "Results saved to $BENCH_DIR/results.txt"
