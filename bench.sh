#!/bin/bash
# Comprehensive multi-thread benchmark for the compressor
# Tests: 1, 5, 10, 15, 20 threads with 2GB fragments on /home/azam/personal

set -e

INPUT="/home/azam/personal"
FRAG_SIZE=2147483648  # 2 GB
BINARY="./target/release/compressor"
THREADS=(1 5 10 15 20)

echo "═══════════════════════════════════════════════════════════════════" | tee benchmark_results.txt
echo " COMPREHENSIVE BENCHMARK — $(du -sh $INPUT 2>/dev/null | cut -f1) real data" | tee -a benchmark_results.txt
echo " Fragment size: 2 GB" | tee -a benchmark_results.txt
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

for T in "${THREADS[@]}"; do
    OUTDIR=$(mktemp -d)
    
    # Run with time -v, but stream the output live to the screen in real-time
    # so the user doesn't think it's hung. We capture only the time output via a tmp file.
    TIME_OUT=$(mktemp)
    
    $TIME_CMD -v -o "$TIME_OUT" $BINARY compress "$INPUT" "$OUTDIR" \
        --fragment-size $FRAG_SIZE \
        --threads $T
    
    RESULT=$(cat "$TIME_OUT")
    
    # Extract metrics
    WALL=$(echo "$RESULT" | /usr/bin/grep "Elapsed (wall clock)" | awk '{print $NF}')
    CPU_PCT=$(echo "$RESULT" | /usr/bin/grep "Percent of CPU" | awk '{print $NF}')
    RSS_KB=$(echo "$RESULT" | /usr/bin/grep "Maximum resident" | awk '{print $NF}')
    RSS_MB=$(( RSS_KB / 1024 ))
    
    # Parse wall time to seconds for throughput calculation
    # Format is either m:ss.ss or h:mm:ss
    if echo "$WALL" | /usr/bin/grep -q ":.*:"; then
        # h:mm:ss format
        SECS=$(echo "$WALL" | awk -F: '{print $1*3600 + $2*60 + $3}')
    else
        # m:ss.ss format
        SECS=$(echo "$WALL" | awk -F: '{print $1*60 + $2}')
    fi
    
    # Calculate throughput (40 GB / seconds)
    THROUGHPUT=$(echo "scale=1; 40 / $SECS * 1024" | bc 2>/dev/null || echo "N/A")
    
    # Calculate speedup vs 1 thread
    if [ "$T" = "1" ]; then
        BASELINE_TIME="$SECS"
        SPEEDUP="1.00x"
    else
        SPEEDUP=$(echo "scale=2; $BASELINE_TIME / $SECS" | bc 2>/dev/null || echo "N/A")
        SPEEDUP="${SPEEDUP}x"
    fi
    
    printf "%-8s | %-12s | %-12s | %-12s | %-10s | %-10s\n" \
        "$T" "$WALL" "${CPU_PCT}%" "${RSS_MB} MB" "${THROUGHPUT} MB/s" "$SPEEDUP" | tee -a benchmark_results.txt
    
    # Cleanup
    rm -rf "$OUTDIR" "$TIME_OUT"
done

echo "" | tee -a benchmark_results.txt
echo "═══════════════════════════════════════════════════════════════════" | tee -a benchmark_results.txt
echo " BENCHMARK COMPLETE" | tee -a benchmark_results.txt
echo " Results saved to benchmark_results.txt" | tee -a benchmark_results.txt
echo "═══════════════════════════════════════════════════════════════════" | tee -a benchmark_results.txt
