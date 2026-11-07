#!/usr/bin/env bash
# Coordinates target GTK 4.14, English labels, and the 840x600 PeaZip window.
# This X11 smoke test does not cover every desktop/portal picker configuration.
set -euxo pipefail
if [[ ${1:-} != --session ]]; then
  root=$(cd "$(dirname "$0")/.." && pwd)
  binary=$(realpath "${1:?usage: smoke_linux.sh BINARY ARTIFACT_DIRECTORY [FIXTURE]}")
  mkdir -p "${2:?artifact directory required}"
  artifacts=$(realpath "$2")
  fixture=$(realpath "${3:-$root/tests/fixtures/sample.7z}")
  runtime=$(mktemp -d)
  chmod 700 "$runtime"
  trap 'rm -rf "$runtime"' EXIT
  export XDG_RUNTIME_DIR="$runtime" GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_A11Y=none
  export LANG=C.UTF-8 LC_ALL=C.UTF-8
  xvfb-run -a -s '-screen 0 1280x900x24 -nolisten tcp' \
    dbus-run-session -- bash "$0" --session "$binary" "$artifacts" "$fixture"
  exit
fi
binary=$2
artifacts=$3
fixture=$4
run_dir=$(mktemp -d "$artifacts/run-XXXXXX")
artifacts="$run_dir/artifacts"
mkdir -p "$artifacts" "$run_dir/output" "$run_dir/reference"
printf 'preserve-me\n' > "$run_dir/output/sentinel.txt"
"$binary" > "$artifacts/gtk.log" 2>&1 &
app_pid=$!
cleanup() {
  kill "$app_pid" 2>/dev/null || true
  wait "$app_pid" 2>/dev/null || true
}
trap cleanup EXIT
printf 'app_pid=%s display=%s\n' "$app_pid" "$DISPLAY" > "$artifacts/handles.txt"
printf '%s\n' "$run_dir" > "$3/latest-run-path"
window=$(timeout 20 xdotool search --sync --onlyvisible --name '^PeaZip$' | head -n1)
xdotool windowfocus --sync "$window"
sleep 1
import -window root "$artifacts/01-window.png"
xdotool mousemove 75 25 click 1
picker=$(timeout 20 xdotool search --sync --onlyvisible --name '^Open archive$' | head -n1)
sleep 1
import -window root "$artifacts/02-picker.png"
for attempt in 1 2 3; do
  xdotool windowfocus --sync "$picker"
  sleep 1
  xdotool key ctrl+l
  sleep 1
  xdotool mousemove 360 23 click 1 key ctrl+a
  xdotool type --clearmodifiers --delay 40 "$fixture"
  sleep 1
  import -window root "$artifacts/02-location-$attempt.png"
  xdotool key Return
  sleep 2
  if xdotool search --onlyvisible --name '^Open archive$' >/dev/null; then
    xdotool mousemove 775 350 click 1
  fi
  for n in $(seq 1 30); do
    xdotool search --onlyvisible --name '^Open archive$' >/dev/null || break
    sleep .1
  done
  xdotool search --onlyvisible --name '^Open archive$' >/dev/null || break
done
if xdotool search --onlyvisible --name '^Open archive$' >/dev/null; then
  echo 'Archive picker did not close' >&2
  exit 1
fi
sleep 1
import -window root "$artifacts/03-loaded.png"
xdotool mousemove 20 134 click 1
sleep .5
import -window root "$artifacts/04-selected.png"
xdotool mousemove 520 25 click 1
output_picker=$(timeout 20 xdotool search --sync --onlyvisible --name '^Extract selected — choose new folder name$' | head -n1)
xdotool windowfocus --sync "$output_picker"
sleep 1
import -window root "$artifacts/05-output-picker.png"
xdotool mousemove 360 16 click 1 key ctrl+a
xdotool type --clearmodifiers --delay 40 "$run_dir/output/Extracted"
sleep 1
xdotool mousemove 765 350 click 1
for n in $(seq 1 100); do [[ -f "$run_dir/output/Extracted/docs/你好.txt" ]] && break; kill -0 "$app_pid"; sleep .1; done
sleep .5
import -window root "$artifacts/06-extracted.png"
7z x -y "-o$run_dir/reference" "$fixture" > "$artifacts/reference.log"
diff -r "$run_dir/reference/docs" "$run_dir/output/Extracted/docs"
test ! -e "$run_dir/output/Extracted/other.txt"
printf 'preserve-me\n' | cmp - "$run_dir/output/sentinel.txt"
test -d "$run_dir/output/Extracted/docs/empty"
sha256sum "$run_dir/output/Extracted/docs/nested/data.bin" "$run_dir/output/Extracted/docs/你好.txt" > "$artifacts/sha256.txt"
kill -0 "$app_pid"
if grep -Ei 'panic|critical|segmentation|could not open archive|could not extract archive' "$artifacts/gtk.log"; then exit 1; fi
printf 'GUI_IMPORT_SELECTED_DOCS_BYTES_SENTINEL_OK %s\n' "$run_dir"
