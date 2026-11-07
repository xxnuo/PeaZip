#!/usr/bin/env bash
# Actual GTK extraction scope/path modes, verified against independent 7z output.
set -euxo pipefail
if [[ ${1:-} != --session ]]; then
  root=$(cd "$(dirname "$0")/.." && pwd)
  binary=$(realpath "${1:?usage: smoke_linux_extract_options.sh BINARY ARTIFACT_DIRECTORY}")
  mkdir -p "${2:?artifact directory required}"
  output=$(realpath "$2")
  runtime=$(mktemp -d)
  chmod 700 "$runtime"
  trap 'rm -rf "$runtime"' EXIT
  export XDG_RUNTIME_DIR="$runtime" GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_A11Y=none LANG=C.UTF-8 LC_ALL=C.UTF-8
  xvfb-run -a -s '-screen 0 1440x900x24 -nolisten tcp' dbus-run-session -- bash "$0" --session "$binary" "$output" "$root"
  exit
fi
binary=$2 root=$4
run_dir=$(mktemp -d "$3/run-XXXXXX")
artifacts="$run_dir/artifacts" fixture="$run_dir/fixture" outputs="$run_dir/output"
mkdir -p "$artifacts" "$fixture" "$outputs" "$run_dir/collision-input/first" "$run_dir/collision-input/second" "$run_dir/collision-input/empty"
cp "$root/tests/fixtures/sample.7z" "$fixture/sample.7z"
printf 'first copy\n' > "$run_dir/collision-input/first/same.txt"
printf 'second copy\n' > "$run_dir/collision-input/second/same.txt"
(cd "$run_dir/collision-input" && 7z a -tzip "$fixture/collision.zip" first second empty) > "$artifacts/collision-fixture.log"
printf 'preserve extraction sentinel\n' > "$outputs/sentinel.txt"
7z x -y "-o$run_dir/reference" "$fixture/sample.7z" > "$artifacts/reference.log"
mkdir -p "$run_dir/expected-selected-full" "$run_dir/expected-all-files" "$run_dir/expected-selected-files"
cp -a "$run_dir/reference/docs" "$run_dir/expected-selected-full/"
cp "$run_dir/reference/docs/nested/data.bin" "$run_dir/reference/docs/你好.txt" "$run_dir/reference/other.txt" "$run_dir/expected-all-files/"
cp "$run_dir/reference/docs/nested/data.bin" "$run_dir/reference/docs/你好.txt" "$run_dir/expected-selected-files/"
"$binary" > "$artifacts/gtk.log" 2>&1 &
app_pid=$!
trap 'kill "$app_pid" 2>/dev/null || true; wait "$app_pid" 2>/dev/null || true' EXIT
printf '%s\n' "$run_dir" > "$3/latest-run-path"
window=$(timeout 20 xdotool search --sync --onlyvisible --name '^PeaZip$' | head -n1)
width=1440 height=900
resize() {
  width=$1 height=$2
  xdotool windowmove "$window" 0 0 windowsize --sync "$window" "$width" "$height"
  xdotool windowfocus --sync "$window"
  sleep .5
}
screen() {
  import -window root -crop "${width}x${height}+0+0" "$artifacts/$1.png"
}
ocr() {
  screen current
  convert "$artifacts/current.png" -resize 300% "$artifacts/ocr.png"
  tesseract "$artifacts/ocr.png" "$artifacts/current" --psm 11 tsv 2> "$artifacts/ocr.log"
}
# The last matching word can distinguish table docs from sidebar paths, and
# native dialog buttons from the toolbar behind them. Coordinates are unscaled.
locate() {
  awk -F '\t' -v word="$1" -v minx="${2:-0}" -v miny="${3:-0}" -v maxy="${4:-$height}" -v maxx="${5:-$width}" -v first="${6:-0}" '
    $12 ~ word && $7/3 >= minx && $8/3 >= miny && $8/3 <= maxy && $7/3 <= maxx {if(!found || !first || $8<top) {x=int(($7+$9/2)/3); y=int(($8+$10/2)/3); top=$8; found=1}}
    END {if(found) print x,y; else exit 1}' "$artifacts/current.tsv"
}
click_word() {
  ocr
  read -r x y < <(locate "$@")
  xdotool mousemove "$x" "$y" click 1
  sleep .4
}
menu_open() {
  xdotool key Escape
  ocr
  read -r menu_x menu_y < <(locate "$1" 0 0 35)
  xdotool mousemove "$menu_x" "$menu_y" click 1
  sleep .4
}
menu_action() {
  menu_open "$1"
  screen "menu-$1-before-$2"
  # Native popovers anchor below their top-level label. Restrict OCR to that
  # popup so an identical toolbar label behind it cannot invoke the command.
  ocr
  read -r x y < <(locate "$2" 0 "$((menu_y+12))" 420 "$((menu_x+200))" 1)
  xdotool mousemove "$x" "$y" click 1
  sleep .5
}
selection_count() {
  ocr
  read -r selected_x selected_y < <(locate selected)
  convert "$artifacts/current.png" -crop "${width}x30+0+$((selected_y-10))" +repage -resize 300% "$artifacts/selection.png"
  tesseract "$artifacts/selection.png" "$artifacts/selection" --psm 6 2> "$artifacts/selection-ocr.log"
  grep -Eq "(^| )$1 selected" "$artifacts/selection.txt"
}
status_text() {
  screen current
  convert "$artifacts/current.png" -crop "${width}x40+0+$((height-40))" +repage -resize 300% "$artifacts/status.png"
  tesseract "$artifacts/status.png" "$artifacts/status" --psm 6 2> "$artifacts/status-ocr.log"
}
capture_sizes() {
  for dimensions in '1024 600' '1440 900'; do
    read -r w h <<< "$dimensions"
    resize "$w" "$h"
    screen "${w}x${h}-$1"
  done
  resize 1440 900
}
wait_status() {
  for n in $(seq 1 20); do
    kill -0 "$app_pid"
    status_text
    grep -Eq "$1" "$artifacts/status.txt" && return
    sleep .2
  done
  cat "$artifacts/status.txt" >&2
  return 1
}
row() {
  ocr
  read -r header_x header_y < <(locate '^Name$')
  locate "$1" 0 "$((header_y+12))" "$((height-45))" "$((width-220))" 1
}
select_row() {
  read -r x y < <(row "$1")
  xdotool mousemove "$x" "$y" click 1
  sleep .3
}
open_row() {
  read -r x y < <(row "$1")
  xdotool mousemove "$((width-18))" "$y" click 1
  sleep .4
}
pass() { printf '%s pass\n' "$1" | tee -a "$artifacts/results.txt"; }
open_archive() {
  local name=$1
  menu_action File '^archive'
  picker=$(timeout 20 xdotool search --sync --onlyvisible --name '^Open archive$' | head -n1)
  xdotool windowfocus --sync "$picker"
  sleep .5
  xdotool key ctrl+l
  sleep .5
  xdotool mousemove 360 23 click 1 key ctrl+a
  xdotool type --clearmodifiers --delay 0 "$fixture/"
  sleep .5
  xdotool key Return
  sleep .6
  geometry=$(xdotool getwindowgeometry --shell "$picker")
  pw=$(awk -F= '$1=="WIDTH"{print $2}' <<< "$geometry")
  ph=$(awk -F= '$1=="HEIGHT"{print $2}' <<< "$geometry")
  ocr
  read -r x y < <(locate "^$name$" 0 35 "$((ph-70))" "$pw" 1)
  xdotool mousemove "$x" "$y" click --repeat 2 --delay 150 1
  wait_status '[0-9]+ entries'
}
modal_word() { click_word "$1" "$((width/2-230))" 100 "$((height-50))" "$((width/2+230))" 1; }
open_extract() {
  menu_action Tools '^Extract'
  ocr
  locate '^Scope$' 0 100 > /dev/null
  locate '^Paths$' 0 100 > /dev/null
}
choose_extract() {
  modal_word "^$1$"
  modal_word "^$2$"
  screen "options-$1-$2"
  modal_word '^Continue$'
}
output_name() {
  local path=$1
  picker=$(timeout 20 xdotool search --sync --onlyvisible --name '^Extract — choose new folder name$' | head -n1)
  xdotool windowfocus --sync "$picker"
  sleep .5
  xdotool mousemove 360 16 click 1 key ctrl+a
  xdotool type --clearmodifiers --delay 0 "$path"
  sleep .5
  xdotool key Return
}
output_state() {
  { find "$outputs" -printf '%y %P\n' | sort; find "$outputs" -type f -exec sha256sum '{}' + | sort; } > "$artifacts/$1.manifest"
}
resize 1440 900
open_archive sample.7z
wait_status '^6 entries'
selection_count 0
selected=0
for variant in all-full all-files selected-full selected-files; do
  case "$variant" in
    all-full) scope=All; paths=Full; expected="$run_dir/reference"; count=6 ;;
    all-files) scope=All; paths=Files; expected="$run_dir/expected-all-files"; count=3 ;;
    selected-full) scope=Selected; paths=Full; expected="$run_dir/expected-selected-full"; count=5 ;;
    selected-files) scope=Selected; paths=Files; expected="$run_dir/expected-selected-files"; count=2 ;;
  esac
  if [[ "$scope" = Selected && "$selected" = 0 ]]; then select_row '^docs$'; selected=1; fi
  selection_count "$selected"
  open_extract
  if [[ "$variant" = all-full ]]; then capture_sizes extract-options-default; fi
  choose_extract "$scope" "$paths"
  test ! -e "$outputs/$variant"
  output_name "$outputs/$variant"
  wait_status "Extracted $count entries"
  diff -r "$expected" "$outputs/$variant"
  if [[ "$paths" = Files ]]; then test "$(find "$outputs/$variant" -mindepth 1 -type d | wc -l)" -eq 0; fi
  selection_count "$selected"
  screen "extracted-$variant"
  pass "$variant-exact-paths-bytes-and-empty-directory-policy"
done
open_archive collision.zip
selection_count 0
row '^first$' > /dev/null
row '^second$' > /dev/null
output_state before-collision
open_extract
choose_extract All Files
output_name "$outputs/collision"
wait_status 'Could not extract archive: duplicate or conflicting archive path:.*same.txt'
test ! -e "$outputs/collision"
selection_count 0
row '^first$' > /dev/null
row '^second$' > /dev/null
output_state after-collision
cmp "$artifacts/before-collision.manifest" "$artifacts/after-collision.manifest"
screen collision-rejected
pass files-only-duplicate-refused-without-output-or-snapshot-change
open_extract
choose_extract All Full
output_name "$outputs/collision"
wait_status 'Extracted 5 entries'
diff -r "$run_dir/collision-input" "$outputs/collision"
printf 'preserve extraction sentinel\n' | cmp - "$outputs/sentinel.txt"
screen full-paths-retry-success
pass full-paths-same-destination-retry-preserves-both-same-names
if grep -Ei 'panic|critical|segmentation' "$artifacts/gtk.log"; then exit 1; fi
printf 'GUI_EXTRACT_SCOPE_PATHS_COLLISION_SAFE_RETRY_OK %s\n' "$run_dir"
