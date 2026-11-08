#!/usr/bin/env bash
# Real GTK TAR/TAR.GZ creation: native save picker and independent readers.
set -euxo pipefail
if [[ ${1:-} != --session ]]; then
  root=$(cd "$(dirname "$0")/.." && pwd)
  binary=$(realpath "${1:?usage: smoke_linux_create_formats.sh BINARY ARTIFACT_DIRECTORY}")
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
mkdir -p "$artifacts" "$fixture/empty" "$fixture/tree/empty" "$outputs" "$run_dir/expected/empty" "$run_dir/expected/tree/empty"
for n in $(seq 1 1024); do printf '%08d alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima\n' "$n"; done > "$fixture/payload.txt"
head -c 1048576 /dev/zero > "$fixture/bomb.bin"
cp "$root/tests/fixtures/sample.7z" "$fixture/sample.7z"
printf 'Unicode payload\n' > "$fixture/tree/你好.txt"
printf '\000\377\001binary\n' > "$fixture/binary.bin"
cp "$fixture/tree/你好.txt" "$run_dir/expected/tree/"
cp "$fixture/payload.txt" "$fixture/binary.bin" "$run_dir/expected/"
printf 'keep this sentinel\n' > "$outputs/sentinel.txt"
sha256sum "$fixture/sample.7z" > "$artifacts/original-archive.sha256"
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
modal_word() { click_word "$1" "$((width/2-230))" 100 "$((height-60))" "$((width/2+230))" 1; }
expect_modal() { ocr; locate '^ZIP$' 0 100 > /dev/null; locate '^Continue$' 0 100 > /dev/null; }
expect_modal_closed() {
  for n in $(seq 1 12); do ocr; if ! locate '^Continue$' 0 100 > /dev/null; then return; fi; sleep .2; done
  screen modal-did-not-close
  return 1
}
open_options() { menu_action File '^Create$'; expect_modal; }
choose_mode() {
  local mode=$1 level=${2:-}
  modal_word "^$mode$"
  if [[ "$mode" = Deflate ]]; then
    ocr
    read -r label_x label_y < <(locate '^level$' "$((width/2-230))" 100 "$((height-60))" "$((width/2+230))" 1)
    coordinates=$(awk -F '\t' -v y="$label_y" -v minx="$label_x" -v maxx="$((width/2+230))" '$12 ~ /^[1-9][|]?$/ && $7/3>minx && $7/3<maxx && $8/3>y-15 && $8/3<y+15 {print int(($7+$9/2)/3),int(($8+$10/2)/3);exit}' "$artifacts/current.tsv")
    test -n "$coordinates"
    read -r x y <<< "$coordinates"
    xdotool mousemove "$x" "$y" click 1 key ctrl+a
    xdotool type --clearmodifiers --delay 0 "$level"
    screen "options-Deflate-$level"
  else
    screen "options-$mode"
  fi
}
file_from_picker() {
  local title=$1 filename=$2
  picker=$(timeout 20 xdotool search --sync --onlyvisible --name "$title" | head -n1)
  xdotool windowfocus --sync "$picker"
  sleep .5
  xdotool key ctrl+l
  sleep .5
  xdotool mousemove 360 23 click 1 key ctrl+a
  xdotool type --clearmodifiers --delay 0 "$fixture/"
  sleep .5
  xdotool key Return
  sleep .7
  picker_geometry=$(xdotool getwindowgeometry --shell "$picker")
  picker_width=$(awk -F= '$1=="WIDTH"{print $2}' <<< "$picker_geometry")
  picker_height=$(awk -F= '$1=="HEIGHT"{print $2}' <<< "$picker_geometry")
  ocr
  read -r x y < <(locate "^$filename$" 0 35 "$((picker_height-70))" "$picker_width" 1)
  xdotool mousemove "$x" "$y" click --repeat 2 --delay 150 1
  sleep .5
}
save_to() {
  local destination=$1
  save=$(timeout 20 xdotool search --sync --onlyvisible --name '^Create archive — choose a new filename$' | head -n1)
  xdotool windowfocus --sync "$save"
  sleep .5
  xdotool mousemove 360 16 click 1 key ctrl+a
  xdotool type --clearmodifiers --delay 0 "$destination"
  sleep .5
  xdotool key Return
}
open_original() {
  menu_action File '^archive'
  file_from_picker '^Open archive$' 'sample.7z'
  wait_status '^6 entries'
  select_row '^docs$'
  selection_count 1
}
original_unchanged() {
  row '^docs$' > /dev/null
  row '^other.txt$' > /dev/null
  selection_count 1
  sha256sum -c "$artifacts/original-archive.sha256"
}
output_manifest() { find "$outputs" -type f -exec sha256sum '{}' + | sort > "$artifacts/$1.manifest"; }
table_snapshot() {
  ocr
  read -r x y < <(locate '^Name$')
  convert "$artifacts/current.png" -crop "${width}x$((height-y-55))+0+$((y+15))" +repage "$artifacts/$1-table.png"
}
fixture_view() {
  xdotool key ctrl+l
  sleep .3
  xdotool type --clearmodifiers --delay 0 "$fixture"
  xdotool key Return
  wait_status '^6 items'
}
choose_format() {
 modal_word "^$1$"
 screen "format-$1"
}
choose_gzip_level() {
 ocr
 read -r lx ly < <(locate '^level$' "$((width/2-230))" 100 "$((height-60))" "$((width/2+230))" 1)
 read -r x y < <(awk -F '\t' -v y="$ly" -v minx="$lx" -v maxx="$((width/2+230))" '$12 ~ /^[1-9][|]?$/ && $7/3>minx && $7/3<maxx && $8/3>y-15 && $8/3<y+15 {print int(($7+$9/2)/3),int(($8+$10/2)/3);exit}' "$artifacts/current.tsv")
 xdotool mousemove "$x" "$y" click 1 key ctrl+a
 xdotool type --clearmodifiers --delay 0 "$1"
}
verify_tar() {
 local archive=$1 tag=$2
 mkdir -p "$run_dir/$tag-tar" "$run_dir/$tag-7z"
 tar -tf "$archive" > "$artifacts/$tag-tar-list.txt"
 tar -xf "$archive" -C "$run_dir/$tag-tar"
 diff -r "$run_dir/expected" "$run_dir/$tag-tar"
 if [[ "$archive" == *.gz ]]; then
  gzip -t "$archive"
  gzip -dc "$archive" > "$run_dir/$tag.tar"
  7z x -y "-o$run_dir/$tag-7z" "$run_dir/$tag.tar" > "$artifacts/$tag-7z.txt"
 else
  7z x -y "-o$run_dir/$tag-7z" "$archive" > "$artifacts/$tag-7z.txt"
 fi
 diff -r "$run_dir/expected" "$run_dir/$tag-7z"
}
resize 1440 900
open_original
output_manifest initial
open_options
choose_format TAR
capture_sizes create-tar-options
modal_word '^Cancel$'
expect_modal_closed
original_unchanged
output_manifest cancelled
cmp "$artifacts/initial.manifest" "$artifacts/cancelled.manifest"
pass format-cancel-no-output-original-selection-preserved
menu_action File '^folder'
picker=$(timeout 20 xdotool search --sync --onlyvisible --name '^Open folder$' | head -n1)
xdotool windowfocus --sync "$picker"
sleep .5
xdotool key ctrl+l
sleep .4
xdotool mousemove 360 23 click 1 key ctrl+a
xdotool type --clearmodifiers --delay 0 "$fixture/"
xdotool key Return
sleep .7
if xdotool search --onlyvisible --name '^Open folder$' >/dev/null; then click_word '^Browse$' 0 150; fi
wait_status '^6 items'
for format in TAR TAR.GZ; do
 select_row '^payload.txt$'
 select_row '^binary.bin$'
 select_row '^empty$'
 select_row '^tree$'
 selection_count 4
 open_options
 choose_format "$format"
 if [[ "$format" == TAR.GZ ]]; then choose_gzip_level 9; fi
 capture_sizes "create-$format-options"
 modal_word '^Continue$'
 ! xdotool search --onlyvisible --name '^Select files to archive$' >/dev/null
 extension=tar
 if [[ "$format" == TAR.GZ ]]; then extension=tar.gz; fi
 save_to "$outputs/created.$extension"
 wait_status "Created $format: 6 entries"
 verify_tar "$outputs/created.$extension" "$format"
 capture_sizes "created-$format"
 pass "$format-independent-tar-gzip-7z-bytes-empty-unicode-binary"
 menu_action File '^Close$'
 fixture_view
done
# Keep an archive snapshot and its checked docs row across the failed gzip task.
open_original
output_manifest before-ratio
table_snapshot before-ratio
open_options
choose_format TAR.GZ
choose_gzip_level 9
modal_word '^Continue$'
file_from_picker '^Select files to archive$' 'bomb.bin'
save_to "$outputs/ratio.tar.gz"
wait_status 'Could not create.*compression ratio'
original_unchanged
table_snapshot after-ratio
compare -metric AE "$artifacts/before-ratio-table.png" "$artifacts/after-ratio-table.png" null: 2> "$artifacts/ratio-table-diff.txt"
test ! -e "$outputs/ratio.tar.gz"
output_manifest after-ratio
cmp "$artifacts/before-ratio.manifest" "$artifacts/after-ratio.manifest"
screen gzip-ratio-refused
pass gzip-ratio-error-preserves-snapshot-selection-output
open_options
choose_format TAR
modal_word '^Continue$'
file_from_picker '^Select files to archive$' 'bomb.bin'
save_to "$outputs/ratio.tar"
wait_status 'Created TAR: 1 entries'
mkdir -p "$run_dir/retry"
tar -xf "$outputs/ratio.tar" -C "$run_dir/retry"
cmp "$fixture/bomb.bin" "$run_dir/retry/bomb.bin"
pass retry-with-uncompressed-tar-bytes
# The GTK save picker may ask Replace; the application must still refuse publication.
open_original
output_manifest before-existing
open_options
choose_format TAR
modal_word '^Continue$'
file_from_picker '^Select files to archive$' 'payload.txt'
save_to "$outputs/sentinel.txt"
for n in $(seq 1 20); do
 if ! xdotool search --onlyvisible --name '^Create archive — choose a new filename$' >/dev/null; then break; fi
 ocr
 if coords=$(locate '^Replace$'); then
  read -r x y <<< "$coords"
  xdotool mousemove "$x" "$y" click 1
 fi
 sleep .3
done
! xdotool search --onlyvisible --name '^Create archive — choose a new filename$' >/dev/null
wait_status 'Could not create.*(exists|exist)'
original_unchanged
output_manifest after-existing
cmp "$artifacts/before-existing.manifest" "$artifacts/after-existing.manifest"
printf 'keep this sentinel\n' | cmp - "$outputs/sentinel.txt"
pass existing-target-refused-no-overwrite
if grep -Ei 'panic|critical|segmentation' "$artifacts/gtk.log"; then exit 1; fi
printf 'GUI_CREATE_TAR_TARGZ_CANCEL_BYTES_RATIO_RETRY_NO_OVERWRITE_OK %s\n' "$run_dir"
