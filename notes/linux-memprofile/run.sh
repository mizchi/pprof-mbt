#!/usr/bin/env bash
# End-to-end Linux verification for `moon-pprof memprofile-native`.
#
# Run from the repo root:
#   bash notes/linux-memprofile/run.sh
#
# What this does:
#   1. Build the moon-pprof-linux image if missing.
#   2. cargo build --release inside the container (artefacts kept in a
#      named volume so it doesn't fight macOS host's target/).
#   3. moon build the workload at notes/linux-memprofile/workload.
#   4. moon-pprof memprofile-native <main.exe> --out linux-mem.pb.gz
#   5. moon-pprof summary linux-mem.pb.gz — should list MoonBit
#      symbols (alloc_strings / alloc_arrays / StringBuilder / Array).
set -euo pipefail

REPO_ROOT=$(git rev-parse --show-toplevel)
IMAGE=moon-pprof-linux:latest
TARGET_VOLUME=moon-pprof-linux-target
WORKLOAD_DIR=/work/notes/linux-memprofile/workload
OUT_PROFILE=$WORKLOAD_DIR/linux-mem.pb.gz

if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "==> building $IMAGE"
  docker build -t "$IMAGE" -f "$REPO_ROOT/notes/linux-memprofile/Dockerfile" "$REPO_ROOT"
fi

echo "==> running verification inside $IMAGE"
docker run --rm \
  --platform linux/aarch64 \
  -v "$REPO_ROOT":/work \
  -v "$TARGET_VOLUME":/work/target \
  -w /work \
  -e CARGO_TARGET_DIR=/work/target \
  "$IMAGE" \
  bash -euo pipefail -c '
echo "==> cargo build --release -p moon-pprof"
cargo build --release -p moon-pprof

MOON_PPROF=/work/target/release/moon-pprof

echo "==> moon build workload (native, release)"
cd '"$WORKLOAD_DIR"'
moon clean
moon build --target native --release

EXE=$(find _build -type f -name "main.exe" | head -1)
if [[ -z "$EXE" ]]; then
  echo "FAIL: did not find main.exe under _build/" >&2
  find _build -maxdepth 6 -type f >&2
  exit 1
fi
echo "==> built $EXE"

echo "==> moon-pprof memprofile-native"
"$MOON_PPROF" memprofile-native "$EXE" --out '"$OUT_PROFILE"'

echo "==> moon-pprof summary"
"$MOON_PPROF" summary '"$OUT_PROFILE"'
'
