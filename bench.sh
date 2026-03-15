#!/bin/bash
# Comprehensive multi-thread benchmark for the compressor
# Tests: 1, 5, 10, 15, 20 threads with 2GB fragments on /home/azam/personal

set -e

INPUT="/home/azam/personal"
FRAG_SIZE=16777216  # 2 GB
BINARY="./target/release/compressor"
THREADS=(1 5 10 15 20)

echo "═══════════════════════════════════════════════════════════════════" | tee benchmark_results.txt
echo " COMPREHENSIVE BENCHMARK — $(du -sh $INPUT 2>/dev/null | cut -f1) real data" | tee -a benchmark_results.txt
echo " Fragment size: 16 MB" | tee -a benchmark_results.txt
echo " Thread counts: ${THREADS[*]}" | tee -a benchmark_results.txt
echo "═══════════════════════════════════════════════════════════════════" | tee -a benchmark_results.txt
echo "" | tee -a benchmark_results.txt

# Header for results table
printf "%-8s | %-12s | %-12s | %-12s | %-10s | %-10s\n" \
    "Threads" "Wall Time" "CPU Usage" "Peak RAM" "Throughput" "Speedup" | tee -a benchmark_results.txt
printf "%-8s-+-%-12s-+-%-12s-+-%-12s-+-%-10s-+-%-10s\n" \
    "--------" "------------" "------------" "------------" "----------" "----------" | tee -a benchmark_results.txt

# Find time command (Termux has it in a different path)
TIME_CMD=$(command -v time || echo "/usr/bin/time")

# Calculate throughput (Total bytes / seconds)
TOTAL_KB=$(du -s "$INPUT" | awk '{print $1}')

for T in "${THREADS[@]}"; do
    # Run with time -v, but stream the output live to the screen in real-time
    # so the user doesn't think it's hung. We capture only the time output via a tmp file.
    # Cap threads to number of fragments if needed
    # (Optional optimization if frag_size is huge)
    
    TMP_ARCHIVE=$(mktemp -d)
    
    printf "%-8s | " "$T" | tee -a benchmark_results.txt
    
    # Run compression with progress bar
    # We use -v to get Peak RAM and Wall Time from the 'time' command
    TIME_OUT=$(mktemp)
    $TIME_CMD -v -o "$TIME_OUT" $BINARY compress "$INPUT" "$TMP_ARCHIVE" --fragment-size $FRAG_SIZE --threads $T
    
    # Parse results from the 'time' output
    WALL_TIME=$(grep "Elapsed (wall clock) time" "$TIME_OUT" | awk '{print $NF}')
    CPU_USAGE=$(grep "Percent of CPU this job got" "$TIME_OUT" | awk '{print $NF}')
    PEAK_RAM=$(grep "Maximum resident set size" "$TIME_OUT" | awk '{print $NF}')
    
    # Rough seconds conversion for throughput
    SEC=$(echo "$WALL_TIME" | awk -F: '{if (NF==3) print $1*3600+$2*60+$3; else print $1*60+$2}')
    THROUGHPUT=$(echo "$TOTAL_KB / 1024 / $SEC" | bc -l | xargs printf "%.1f MB/s")
    
    # Speedup (relative to first run)
    if [ "$T" -eq "${THREADS[0]}" ]; then
        T1_SEC=$SEC
        SPEEDUP="1.0x"
    else
        SPEEDUP=$(echo "$T1_SEC / $SEC" | bc -l | xargs printf "%.2fx")
    fi
    
    # Print and Log row
    printf "%-12s | %-12s | %-12s | %-10s | %-10s\n" "$WALL_TIME" "$CPU_USAGE" "$((PEAK_RAM/1024)) MB" "$THROUGHPUT" "$SPEEDUP" | tee -a benchmark_results.txt
    
    # Run a quick verify decompression (optional but nice for the user to see progress)
    # Uncomment if you want to benchmark decompression too
    # echo "  [Verifying decompression...]"
    # $BINARY decompress "$TMP_ARCHIVE" "/tmp/verify_$$"
    
    # Cleanup
    rm -rf "$TMP_ARCHIVE"
    rm "$TIME_OUT"
done

echo "" | tee -a benchmark_results.txt
echo "═══════════════════════════════════════════════════════════════════" | tee -a benchmark_results.txt
echo " BENCHMARK COMPLETE" | tee -a benchmark_results.txt
echo " Results saved to benchmark_results.txt" | tee -a benchmark_results.txt
echo "═══════════════════════════════════════════════════════════════════" | tee -a benchmark_results.txt
