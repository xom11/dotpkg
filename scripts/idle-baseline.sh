#!/bin/sh
# idle-baseline.sh -- measure what "idle" costs on THIS machine, so that
# scripts/idle-gate.sh's thresholds are derived rather than guessed. The Unix
# half of scripts/idle-baseline.ps1; see that file for what the same numbers
# looked like on the Windows machine.
#
# Run it, read the range it prints, and set IDLE_MAX_MACHINE_PCT to roughly
# three times the maximum. Record what it printed next to whatever the gate then
# admitted -- a threshold with no measurement beside it is the thing this
# project keeps paying for.

set -eu

ROUNDS="${BASELINE_ROUNDS:-3}"
WINDOW="${BASELINE_WINDOW:-6}"
TOP="${BASELINE_TOP:-6}"

case "$(uname -s)" in
    Darwin) CORES=$(sysctl -n hw.logicalcpu) ;;
    *)      CORES=$(nproc 2>/dev/null || echo 1) ;;
esac
[ "$CORES" -ge 1 ] || CORES=1

echo "host           : $(hostname)"
echo "logical_cores  : $CORES"
echo "rounds         : $ROUNDS x ${WINDOW}s"

snapshot() {
    ps -Ao pid=,time=,comm= 2>/dev/null | awk '
        {
            t = $2; days = 0
            if (index(t, "-") > 0) { split(t, d, "-"); days = d[1]; t = d[2] }
            n = split(t, f, ":"); secs = 0
            for (i = 1; i <= n; i++) secs = secs * 60 + f[i]
            secs += days * 86400
            name = $3; sub(/.*\//, "", name)
            print $1, secs, name
        }'
}

BUSIES=''
round=1
while [ "$round" -le "$ROUNDS" ]; do
    start=$(date +%s)
    s1=$(snapshot)
    sleep "$WINDOW"
    s2=$(snapshot)
    elapsed=$(( $(date +%s) - start ))
    [ "$elapsed" -ge 1 ] || elapsed=1

    out=$(printf '%s\n' "$s1" "$s2" | awk \
        -v split_at="$(printf '%s\n' "$s1" | wc -l)" \
        -v elapsed="$elapsed" -v cores="$CORES" -v top="$TOP" '
        NR <= split_at { before[$1] = $2; next }
        {
            d = $2 - ($1 in before ? before[$1] : 0)
            if (d <= 0) next
            total += d
            rows[++n] = sprintf("  %s %.2f cpu-s = %.1f%% of one core", $3, d, 100 * d / elapsed)
            vals[n] = d
        }
        END {
            printf "BUSY %.2f %.2f\n", 100 * total / (elapsed * cores), total
            for (i = 1; i <= n; i++) for (j = i + 1; j <= n; j++)
                if (vals[j] > vals[i]) { t = vals[i]; vals[i] = vals[j]; vals[j] = t
                                         s = rows[i]; rows[i] = rows[j]; rows[j] = s }
            for (i = 1; i <= n && i <= top; i++) print rows[i]
        }')

    busy=$(printf '%s\n' "$out" | awk '$1 == "BUSY" { print $2 }')
    totalcpu=$(printf '%s\n' "$out" | awk '$1 == "BUSY" { print $3 }')
    BUSIES="$BUSIES $busy"
    echo "--- round $round  window=${elapsed}s ---"
    echo "  machine_busy_pct     : $busy   (total $totalcpu cpu-s over $CORES cores)"
    printf '%s\n' "$out" | grep -v '^BUSY '
    round=$(( round + 1 ))
done

echo ''
printf 'machine_busy_pct range : %s\n' "$(printf '%s\n' $BUSIES | sort -n | awk 'NR==1 { min=$1 } { max=$1 } END { print min " .. " max }')"
echo 'Set IDLE_MAX_MACHINE_PCT to roughly three times the maximum above.'
