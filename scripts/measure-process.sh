#!/bin/sh
# Usage: sh scripts/measure-process.sh PID [SECONDS]. Read-only Linux process sampling.
set -eu
task_pid="$1"
task_seconds="${2:-60}"
case "$task_pid:$task_seconds" in *[!0-9:]*|:*|*:) exit 2;; esac
task_ticks=$(getconf CLK_TCK)
task_start=$(awk '{print $14+$15}' "/proc/$task_pid/stat")
task_rss_peak=0
task_rss_sum=0
task_count=0
task_minimized_samples=0
while [ "$task_count" -lt "$task_seconds" ]; do
    task_rss=$(awk '/^VmRSS:/ {print $2}' "/proc/$task_pid/status")
    task_rss_sum=$((task_rss_sum + task_rss))
    if [ "$task_rss" -gt "$task_rss_peak" ]; then task_rss_peak="$task_rss"; fi
    if [ -n "${PIKKER_MEASURE_WINDOW:-}" ]; then
        if xprop -id "$PIKKER_MEASURE_WINDOW" WM_STATE 2>/dev/null | rg 'window state: Iconic' >/dev/null; then
            task_minimized_samples=$((task_minimized_samples+1))
        fi
    fi
    sleep 1
    task_count=$((task_count+1))
done
task_end=$(awk '{print $14+$15}' "/proc/$task_pid/stat")
awk -v ticks="$task_ticks" -v duration="$task_seconds" -v delta="$((task_end-task_start))" -v rss="$task_rss_sum" -v peak="$task_rss_peak" 'BEGIN {printf "seconds=%d cpu_percent_one_core=%.4f rss_mean_mib=%.3f rss_peak_mib=%.3f cpu_ticks=%d\n", duration, delta/ticks/duration*100, rss/duration/1024, peak/1024, delta}'
if [ -n "${PIKKER_MEASURE_WINDOW:-}" ]; then
    printf 'minimized_samples=%s/%s\n' "$task_minimized_samples" "$task_seconds"
    [ "$task_minimized_samples" -eq "$task_seconds" ]
fi
