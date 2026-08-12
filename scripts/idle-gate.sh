#!/bin/sh
# idle-gate.sh -- refuse to start a long measurement on a machine that is not idle.
#
# The Unix half of scripts/idle-gate.ps1, and it exists because the standing
# rule ("cargo mutants runs on an idle machine with nothing editing the tree")
# covers both platforms while the gate covered one. The round that built the
# Windows half then ran its own macOS mutation run on a machine measured NOT
# idle -- syspolicyd at 100% of one core, from that round's own builds -- and
# said so in prose instead of being stopped. That is the shape this file exists
# to remove.
#
# It decides the same way the PowerShell one does, deliberately: sample the
# process table twice and decide on the CPU-seconds burned BETWEEN the samples,
# not on an instantaneous load figure. Load averages on macOS are a decayed
# 1-minute mean and would still be reporting a build that finished thirty
# seconds ago; `ps` CPU-time deltas report what is running now.
#
# Exit 0: the caller may proceed.  Exit 1: it must not.
#
# Thresholds are MEASURED, not chosen -- run scripts/idle-baseline.sh on the
# machine in question and set them from what it prints. The defaults below come
# from this project's macOS machine (10 logical cores); see the table in
# docs/measurements-2026-08-12-phase6-citations.md.

set -eu

WINDOW="${IDLE_WINDOW:-6}"
MAX_MACHINE_PCT="${IDLE_MAX_MACHINE_PCT:-10}"
MAX_PROCESS_CORE_PCT="${IDLE_MAX_PROCESS_CORE_PCT:-35}"
EXPECT="${IDLE_EXPECT:-}"
REPORT_ONLY="${IDLE_REPORT_ONLY:-0}"

case "$(uname -s)" in
    Darwin) CORES=$(sysctl -n hw.logicalcpu) ;;
    *)      CORES=$(nproc 2>/dev/null || echo 1) ;;
esac
[ "$CORES" -ge 1 ] || CORES=1

# Names that ONLY exist while something is being compiled or tested. Presence
# alone is disqualifying for these: a linker between two translation units is
# idle for a moment and the machine is still in use.
#
# `node`, `python` and their kin are deliberately NOT here, and the reason was
# measured rather than guessed: the first version listed them, and it refused
# this machine because the editor session itself runs node. A long-lived runtime
# that happens to be resident is not a build, so those are left to the CPU
# threshold below, which is the signal that actually distinguishes them.
BUILDERS='cargo rustc rustdoc cc1 cc1plus clang gcc ld lld make ninja cmake swift-frontend xcodebuild'

snapshot() {
    # pid and cumulative CPU time, one per line, seconds as a float.
    ps -Ao pid=,time=,comm= 2>/dev/null | awk '
        {
            t = $2
            days = 0
            if (index(t, "-") > 0) { split(t, d, "-"); days = d[1]; t = d[2] }
            n = split(t, f, ":")
            secs = 0
            for (i = 1; i <= n; i++) secs = secs * 60 + f[i]
            secs += days * 86400
            name = $3
            sub(/.*\//, "", name)
            print $1, secs, name
        }'
}

STAMP_START=$(date +%s)
SNAP1=$(snapshot)
sleep "$WINDOW"
SNAP2=$(snapshot)
ELAPSED=$(( $(date +%s) - STAMP_START ))
[ "$ELAPSED" -ge 1 ] || ELAPSED=1

REPORT=$(printf '%s\n' "$SNAP1" "$SNAP2" | awk \
    -v split_at="$(printf '%s\n' "$SNAP1" | wc -l)" \
    -v elapsed="$ELAPSED" -v cores="$CORES" \
    -v maxproc="$MAX_PROCESS_CORE_PCT" -v expect="$EXPECT" -v builders="$BUILDERS" '
    BEGIN { nb = split(builders, B, " "); ne = split(expect, E, " ") }
    function allowed(name,   i) {
        for (i = 1; i <= ne; i++) if (E[i] != "" && index(name, E[i]) == 1) return 1
        return 0
    }
    NR <= split_at { before[$1] = $2; next }
    {
        pid = $1; now = $2; name = $3
        d = now - (pid in before ? before[pid] : 0)
        if (d <= 0) next
        total += d
        corepct = 100 * d / elapsed
        isbuilder = 0
        for (i = 1; i <= nb; i++) if (name == B[i]) isbuilder = 1
        if (isbuilder && !allowed(name)) printf "BUILDER\t%s\t%d\n", name, pid
        if (corepct > maxproc) {
            printf "%s\t%s\t%d\t%.2f\t%.1f\n", (allowed(name) ? "allowed" : "BLOCKING"), name, pid, d, corepct
        }
    }
    END { printf "TOTAL\t%.2f\t%.2f\n", total, 100 * total / (elapsed * cores) }
')

MACHINE_PCT=$(printf '%s\n' "$REPORT" | awk -F'\t' '$1 == "TOTAL" { print $3 }')
TOTAL_CPU=$(printf '%s\n' "$REPORT" | awk -F'\t' '$1 == "TOTAL" { print $2 }')
BLOCKERS=$(printf '%s\n' "$REPORT" | awk -F'\t' '$1 == "BLOCKING" { n++ } END { print n + 0 }')
BUILDERS_ALIVE=$(printf '%s\n' "$REPORT" | awk -F'\t' '$1 == "BUILDER" { n++ } END { print n + 0 }')

echo '--- idle-gate ---'
echo "sampled_at          : $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "window_seconds      : $ELAPSED"
echo "logical_cores       : $CORES"
echo "machine_busy_pct    : $MACHINE_PCT   (total $TOTAL_CPU cpu-s)"
echo "expect_allowed      : ${EXPECT:-<none>}"
echo "threshold_machine   : $MAX_MACHINE_PCT %"
echo "threshold_process   : $MAX_PROCESS_CORE_PCT % of one core"
printf '%s\n' "$REPORT" | awk -F'\t' '
    $1 == "BLOCKING" || $1 == "allowed" { printf "  %s %s (pid %s) %s cpu-s = %s%% of one core\n", $1, $2, $3, $4, $5 }
    $1 == "BUILDER" { printf "  BLOCKING %s (pid %s) -- build/test process\n", $2, $3 }'

REASONS=''
over=$(awk -v a="$MACHINE_PCT" -v b="$MAX_MACHINE_PCT" 'BEGIN { print (a > b) ? 1 : 0 }')
[ "$over" -eq 1 ] && REASONS="machine busy ${MACHINE_PCT}% exceeds ${MAX_MACHINE_PCT}%"
[ "$BLOCKERS" -gt 0 ] && REASONS="${REASONS}${REASONS:+; }$BLOCKERS process(es) working"
[ "$BUILDERS_ALIVE" -gt 0 ] && REASONS="${REASONS}${REASONS:+; }$BUILDERS_ALIVE build/test process(es) alive"

echo ''
if [ -n "$REASONS" ]; then
    echo "VERDICT: NOT IDLE -- $REASONS"
    if [ "$REPORT_ONLY" = "1" ]; then
        echo 'idle_gate=REFUSE (report-only, not enforced)'
        exit 0
    fi
    echo 'idle_gate=REFUSE'
    exit 1
fi

echo 'VERDICT: IDLE'
echo 'idle_gate=OK'
exit 0
