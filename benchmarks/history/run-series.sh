#!/usr/bin/env bash
#
# Measure the whole benchmark-history series (every release x every shape) on
# dedicated bare-metal spot instances.
#
# Measurement is 1-up: one benchmark process, pinned to one core, with the rest
# of the box idle. That is bench-on-metal.sh's own pattern and the reason to
# defer to it — concurrent benchmark processes contend for LLC and memory
# bandwidth, which depresses throughput (measured at 20-25% for unrelated
# overlapping suites, and enough at 32-way self-concurrency to widen this
# series' core-to-core spread to p90 ~9.4%).
#
# Parallelism therefore comes from using several boxes, not several cores: the
# binaries are dealt across instances, each measuring its share sequentially.
#
#   run-series.sh --bins <dir> --out <dir> [--boxes N] [--runs R]
#                 [--measurement-time S] [--region R] [--subnet subnet-...] [--keep]
#
# BENCH_SUBNETS may hold a space-separated subnet list to cycle when capacity is
# short; without it the runner lets each attempt auto-discover one.
#
# Instance ids are appended to <out>/instances.txt as they are created, so a
# crashed dispatcher still leaves a list to clean up by hand; every box also
# carries bench-on-metal's own `shutdown -h` dead-man's switch.

set -uo pipefail

BINS=""
OUT=""
BOXES=4
RUNS=4
MEASURE=4
SUBNET=""
KEEP=0
REGION=""
PROVISION_TRIES=6
# Subnets to cycle when no --subnet is pinned. Bare-metal spot capacity moves
# between AZs and, when a region runs dry, between regions, so a single pinned
# AZ turns a transient shortage into a failed run. Set BENCH_SUBNETS to a
# space-separated list for the account and region you are using; leaving it
# empty lets the runner auto-discover a public subnet each attempt.

while [ $# -gt 0 ]; do
  case "$1" in
    --bins) BINS="$2"; shift 2;;
    --out) OUT="$2"; shift 2;;
    --boxes) BOXES="$2"; shift 2;;
    --runs) RUNS="$2"; shift 2;;
    --measurement-time) MEASURE="$2"; shift 2;;
    --subnet) SUBNET="$2"; shift 2;;
    --region) REGION="$2"; shift 2;;
    --keep) KEEP=1; shift;;
    *) echo "unknown arg: $1" >&2; exit 2;;
  esac
done

[ -n "$BINS" ] || { echo "--bins is required" >&2; exit 2; }
[ -n "$OUT" ] || { echo "--out is required" >&2; exit 2; }
read -r -a SUBNET_POOL <<<"${BENCH_SUBNETS:-}"

mkdir -p "$OUT"
INSTANCES="$OUT/instances.txt"
: >"$INSTANCES"

log() { echo "[$(date -u +%H:%M:%S)] $*"; }

mapfile -t binaries < <(find "$BINS" -maxdepth 1 -type f -name '*.bench' | sort)
[ "${#binaries[@]}" -gt 0 ] || { echo "no *.bench files in $BINS" >&2; exit 2; }
log "${#binaries[@]} binaries across ${BOXES} boxes, ${RUNS} runs each, 1-up"

cleanup() {
  if [ "$KEEP" = 1 ]; then
    log "boxes LEFT RUNNING (--keep); see $INSTANCES"
    return
  fi
  while read -r id; do
    [ -n "$id" ] || continue
    log "terminating $id"
    bench-on-metal.sh ${REGION:+--region "$REGION"} --terminate-instance "$id" >/dev/null 2>&1 ||
      echo "TERMINATE FAILED for $id — do it by hand" >&2
  done <"$INSTANCES"
}
trap cleanup EXIT INT TERM

# One worker per box: provision with its first binary, then reuse that box for
# the rest of its share. bench-on-metal does the tuning, pinning and capture.
worker() {
  local slot="$1"; shift
  local mine=("$@")
  local wlog="$OUT/box${slot}.log"
  local inst="" b label dest got

  {
    echo "box ${slot}: ${#mine[@]} binaries"
    for b in "${mine[@]}"; do
      label="$(basename "$b")"
      dest="$OUT/raw/${label}"
      mkdir -p "$dest"

      if [ -z "$inst" ]; then
        # c7i.metal-24xl spot capacity comes and goes, and asking several AZs
        # at once makes it worse. Cycle AZs with backoff rather than losing the
        # box's whole share to one unlucky moment.
        local attempt
        for attempt in $(seq 1 "$PROVISION_TRIES"); do
          local sn="$SUBNET"
          if [ -z "$sn" ] && [ "${#SUBNET_POOL[@]}" -gt 0 ]; then
            sn="${SUBNET_POOL[$(((attempt + slot) % ${#SUBNET_POOL[@]}))]}"
          fi
          bench-on-metal.sh --spot --keep ${REGION:+--region "$REGION"} ${sn:+--subnet "$sn"} \
            --binary "$b" --args "--measurement-time ${MEASURE}" --runs "$RUNS" \
            --out "$dest" >"$dest/metal.log" 2>&1
          inst="$(grep -oE 'reuse-instance (i-[0-9a-f]+)' "$dest/metal.log" | head -1 | awk '{print $2}')"
          [ -n "$inst" ] && break
          echo "box ${slot}: provision attempt ${attempt} failed (${sn:-auto}); retrying"
          sleep $((60 + slot * 15))
        done
        if [ -z "$inst" ]; then
          echo "box ${slot}: PROVISION FAILED after ${PROVISION_TRIES} tries (see $dest/metal.log)"
          return 1
        fi
        echo "$inst" >>"$INSTANCES"
        echo "box ${slot}: instance $inst"
      else
        bench-on-metal.sh --reuse-instance "$inst" ${REGION:+--region "$REGION"} \
          --binary "$b" --args "--measurement-time ${MEASURE}" --runs "$RUNS" \
          --out "$dest" >"$dest/metal.log" 2>&1
      fi

      got="$(find "$dest" -name stdout.txt -size +1k | head -1)"
      if [ -n "$got" ]; then
        echo "box ${slot}: ${label} ok"
      else
        echo "box ${slot}: ${label} NO OUTPUT"
      fi
    done
    echo "box ${slot}: done"
  } >>"$wlog" 2>&1
}

# Deal round-robin so each box gets a mix of releases and shapes: a box lost to
# a spot reclaim then costs a spread of cells rather than whole releases.
declare -a share
for ((i = 0; i < ${#binaries[@]}; i++)); do
  slot=$((i % BOXES))
  share[$slot]="${share[$slot]:-} ${binaries[$i]}"
done

pids=()
for ((s = 0; s < BOXES; s++)); do
  # shellcheck disable=SC2086
  worker "$s" ${share[$s]} &
  pids+=("$!")
  sleep 20   # stagger provisioning so the spot requests don't collide
done

fail=0
for p in "${pids[@]}"; do wait "$p" || fail=1; done

log "collecting"
mkdir -p "$OUT/captures"
n=0
while read -r sf; do
  label="$(basename "$(dirname "$sf")")"
  name="${label%.bench}"
  # Each --runs pass is delimited by the runner's own banner; split so the
  # parser sees one capture per pass. It keys by benchmark id, so repeats
  # inside a single file would overwrite rather than accumulate.
  # The sysinfo banner (kernel, CPU model, turbo/governor/pin_core) is printed
  # once, before the first run banner. parse_criterion.py harvests the machine
  # metadata from those lines, so it has to survive the split — copy it into
  # every pass rather than letting it fall off the front.
  awk -v base="$OUT/captures/$name" '
    /^=== run [0-9]+ \/ [0-9]+ :/ {
      n++
      if (pre != "") printf "%s", pre > (base ".run" n ".txt")
      next
    }
    n == 0 { pre = pre $0 "\n"; next }
    { print > (base ".run" n ".txt") }
  ' "$sf"
  n=$((n + 1))
done < <(find "$OUT/raw" -name stdout.txt -size +1k)

log "$n binaries -> $(find "$OUT/captures" -name '*.txt' | wc -l) capture files"
[ "$fail" = 0 ] || { log "at least one box failed; see $OUT/box*.log"; exit 1; }
