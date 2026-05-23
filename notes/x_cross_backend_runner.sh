#!/usr/bin/env bash
# Time bench-x cmds across native / wasm-gc / js. 3-run median.
# Uses the runner's own elapsed time (ms) for wasm-gc and js, and
# `time` for native.

set -euo pipefail

BENCHES=(uuid_parse encoding_utf8 path_normalize plain_datetime_parse base64_encode base64_decode json5_parse)
LABEL="${1:-run}"
OUT="${2:-/tmp/bench-x-$LABEL.tsv}"

BX=/home/user/pprof-mbt/bench-x
RUNNERS=/home/user/pprof-mbt/runners

median3() {
  printf "%s\n%s\n%s\n" "$1" "$2" "$3" | sort -n | sed -n '2p'
}

time_native() {
  # outputs seconds
  local exe=$1
  local samples=()
  for _ in 1 2 3; do
    samples+=( "$( { /bin/bash -c "time $exe > /dev/null" ; } 2>&1 | awk '/^real/{ split($2,a,"m"); split(a[2],b,"s"); print a[1]*60+b[1] }')" )
  done
  median3 "${samples[0]}" "${samples[1]}" "${samples[2]}"
}

time_wasmgc() {
  local wasm=$1
  local samples=()
  for _ in 1 2 3; do
    # runner prints "[wasm-gc] 1 iter in NNN.N ms (no profile)" to stderr
    local out
    out=$(node "$RUNNERS/run-wasm-gc.mjs" "$wasm" /tmp/_x.cpuprofile 1 --no-profile 2>&1 >/dev/null)
    local ms
    ms=$(echo "$out" | grep -oE '[0-9]+\.[0-9]+ ms' | head -1 | awk '{print $1}')
    if [[ -z "$ms" ]]; then echo "FAIL"; return; fi
    samples+=( "$(awk "BEGIN { printf \"%.3f\", $ms / 1000 }")" )
  done
  median3 "${samples[0]}" "${samples[1]}" "${samples[2]}"
}

time_js() {
  local js=$1
  local samples=()
  for _ in 1 2 3; do
    local out
    out=$(node "$RUNNERS/run-js.mjs" "$js" /tmp/_x.cpuprofile 1 --no-profile 2>&1 >/dev/null)
    local ms
    ms=$(echo "$out" | grep -oE '[0-9]+\.[0-9]+ ms' | head -1 | awk '{print $1}')
    if [[ -z "$ms" ]]; then echo "FAIL"; return; fi
    samples+=( "$(awk "BEGIN { printf \"%.3f\", $ms / 1000 }")" )
  done
  median3 "${samples[0]}" "${samples[1]}" "${samples[2]}"
}

echo -e "bench\tnative\twasm-gc\tjs" > "$OUT"

for b in "${BENCHES[@]}"; do
  nat="$BX/_build/native/release/build/cmd/$b/$b.exe"
  wgc="$BX/_build/wasm-gc/release/build/cmd/$b/$b.wasm"
  js="$BX/_build/js/release/build/cmd/$b/$b.js"

  if [[ ! -x "$nat" ]] || [[ ! -f "$wgc" ]] || [[ ! -f "$js" ]]; then
    echo "skip $b (missing artifacts)" >&2
    continue
  fi

  t_nat=$(time_native "$nat")
  t_wgc=$(time_wasmgc "$wgc")
  t_js=$(time_js "$js")

  printf "%s\t%s\t%s\t%s\n" "$b" "$t_nat" "$t_wgc" "$t_js" | tee -a "$OUT"
done
