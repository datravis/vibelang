#!/usr/bin/env bash
set -euo pipefail

# ── VibeLang vs Go vs Python Benchmark ──
# Measures: wall time, peak RSS (memory), CPU usage
# Runs each program N times and reports median

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
VIBE="$ROOT_DIR/compiler/target/debug/vibe"
RUNS=${1:-10}
TMPDIR=$(mktemp -d)
trap "rm -rf $TMPDIR" EXIT

PROGRAMS="hello factorial fibonacci pipeline"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
NC='\033[0m'

# ── Helpers ──

median() {
    sort -n | awk '{a[NR]=$1} END{print a[int((NR+1)/2)]}'
}

# Run a command N times, collect wall time (ms), peak RSS (KB), CPU %
# Uses GNU time for memory + CPU
bench_run() {
    local label="$1"
    shift
    local cmd=("$@")

    local times=()
    local mems=()
    local cpus=()

    for i in $(seq 1 "$RUNS"); do
        # GNU time format: wall_seconds peak_rss_kb cpu_percent
        local output
        output=$( { /usr/bin/time -f "%e %M %P" "${cmd[@]}" > /dev/null; } 2>&1 | tail -1 )
        local wall_s=$(echo "$output" | awk '{print $1}')
        local rss_kb=$(echo "$output" | awk '{print $2}')
        local cpu_pct=$(echo "$output" | awk '{print $3}' | tr -d '%')

        # Convert wall time to milliseconds
        local wall_ms=$(echo "$wall_s" | awk '{printf "%.2f", $1 * 1000}')

        times+=("$wall_ms")
        mems+=("$rss_kb")
        cpus+=("$cpu_pct")
    done

    local med_time=$(printf '%s\n' "${times[@]}" | median)
    local med_mem=$(printf '%s\n' "${mems[@]}" | median)
    local med_cpu=$(printf '%s\n' "${cpus[@]}" | median)

    printf "  %-12s %8s ms  %8s KB  %6s%%\n" "$label" "$med_time" "$med_mem" "$med_cpu"
}

# ── Build phase ──

echo -e "${BOLD}Building all programs...${NC}"
echo ""

# Build Vibe programs
echo -e "  ${BLUE}[vibe]${NC} Compiling..."
for prog in $PROGRAMS; do
    "$VIBE" build --target x86_64-unknown-linux-gnu \
        "$ROOT_DIR/examples/${prog}.vibe" \
        -o "$TMPDIR/${prog}_vibe.o" 2>/dev/null
    cc "$TMPDIR/${prog}_vibe.o" -o "$TMPDIR/${prog}_vibe" -lc 2>/dev/null
done

# Build Go programs
echo -e "  ${GREEN}[go]${NC}   Compiling..."
for prog in $PROGRAMS; do
    go build -o "$TMPDIR/${prog}_go" "$SCRIPT_DIR/go/${prog}.go" 2>/dev/null
done

# Python needs no build
echo -e "  ${YELLOW}[py]${NC}   Interpreted (no build step)"
echo ""

# ── Benchmark phase ──

echo -e "${BOLD}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${BOLD}  Benchmark Results (median of $RUNS runs)${NC}"
echo -e "${BOLD}═══════════════════════════════════════════════════════════════${NC}"

for prog in $PROGRAMS; do
    echo ""
    echo -e "${BOLD}  ── $prog ──${NC}"
    echo -e "  ${BOLD}$(printf '%-12s %11s  %11s  %8s' 'Language' 'Time' 'Peak RSS' 'CPU')${NC}"
    echo "  ──────────────────────────────────────────────────────"

    bench_run "vibe"   "$TMPDIR/${prog}_vibe"
    bench_run "go"     "$TMPDIR/${prog}_go"
    bench_run "python" python3 "$SCRIPT_DIR/python/${prog}.py"
done

echo ""
echo -e "${BOLD}═══════════════════════════════════════════════════════════════${NC}"

# ── Binary size comparison ──

echo ""
echo -e "${BOLD}  ── Binary Sizes ──${NC}"
echo -e "  ${BOLD}$(printf '%-12s %11s  %11s' 'Program' 'Vibe' 'Go')${NC}"
echo "  ──────────────────────────────────────────"

for prog in $PROGRAMS; do
    vibe_size=$(stat -c%s "$TMPDIR/${prog}_vibe" 2>/dev/null || echo 0)
    go_size=$(stat -c%s "$TMPDIR/${prog}_go" 2>/dev/null || echo 0)
    vibe_kb=$(echo "$vibe_size" | awk '{printf "%.1f KB", $1/1024}')
    go_kb=$(echo "$go_size" | awk '{printf "%.1f KB", $1/1024}')
    printf "  %-12s %11s  %11s\n" "$prog" "$vibe_kb" "$go_kb"
done

echo ""
