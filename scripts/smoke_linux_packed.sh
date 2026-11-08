#!/usr/bin/env bash
# Actual GTK Packed sorting, responsive columns, selection and extraction.
set -euxo pipefail
if [[ ${1:-} != --session ]]; then
  root=$(cd "$(dirname "$0")/.." && pwd)
  binary=$(realpath "${1:?usage: smoke_linux_packed.sh BINARY ARTIFACT_DIRECTORY}")
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
mkdir -p "$artifacts" "$fixture" "$outputs" "$run_dir/input/folder"
head -c 4096 /dev/urandom > "$run_dir/input/stored.bin"
for n in $(seq 1 512); do printf '%08d packed size sorting alpha bravo charlie delta\n' "$n"; done > "$run_dir/input/deflated.txt"
(cd "$run_dir/input" && 7z a -tzip -mx=0 "$fixture/packed.zip" folder stored.bin) > "$artifacts/fixture-store.log"
(cd "$run_dir/input" && 7z a -tzip -mm=Deflate -mx=9 "$fixture/packed.zip" deflated.txt) > "$artifacts/fixture-deflate.log"
7z l -slt "$fixture/packed.zip" > "$artifacts/fixture-list.txt"
deflated_packed=$(awk '/^Path = /{p=substr($0,8)} /^Packed Size = / && p=="deflated.txt"{print $4}' "$artifacts/fixture-list.txt")
test "$deflated_packed" -gt 0
test "$deflated_packed" -lt 4096
cp "$root/tests/fixtures/sample.7z" "$fixture/solid.7z"
cp "$root/tests/fixtures/sample.tar.gz" "$fixture/sample.tgz"
cp "$root/tests/fixtures/sample.tar" "$fixture/sample.tar"
sha256sum "$fixture/packed.zip" "$fixture/solid.7z" "$fixture/sample.tgz" "$fixture/sample.tar" > "$artifacts/source-before.sha256"
printf 'retain sentinel\n' > "$outputs/sentinel.txt"
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
  read -r word_left word_width < <(awk -F '\t' '$12=="selected" {x=int($7/3);w=int($9/3)} END{print x,w}' "$artifacts/current.tsv")
  convert "$artifacts/current.png" -crop "$((word_width+40))x30+$((word_left-25))+$((selected_y-10))" +repage -resize 300% "$artifacts/selection.png"
  tesseract "$artifacts/selection.png" "$artifacts/selection" --psm 7 2> "$artifacts/selection-ocr.log"
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
assert_order() {
  local expected=$1 tag=$2
  ocr
  read -r hx hy < <(locate '^Name$')
  type_left=$(awk -F '\t' '$12=="Type"{print int($7/3);exit}' "$artifacts/current.tsv")
  awk -F '\t' -v miny="$((hy+12))" -v maxx="$type_left" '$1==5 && $7/3>180 && $7/3<maxx && $8/3>miny && ($12=="folder" || $12=="stored.bin" || $12=="deflated.txt") {print $8,$12}' "$artifacts/current.tsv" | sort -n | awk '{print $2}' > "$artifacts/$tag-order.txt"
  printf '%s\n' "$expected" | cmp - "$artifacts/$tag-order.txt"
}
packed_value() {
  local name=$1 expected=$2 tag=$3
  ocr
  packed_left=$(awk -F '\t' '$12=="Packed" {print int($7/3);exit}' "$artifacts/current.tsv")
  read -r rx ry < <(row "^$name$")
  convert "$artifacts/current.png" -crop "110x24+$((packed_left-3))+$((ry-9))" +repage -resize 300% "$artifacts/$tag-value.png"
  tesseract "$artifacts/$tag-value.png" "$artifacts/$tag-value" --psm 7 2> "$artifacts/$tag-ocr.log"
  if [[ "$expected" = unknown ]]; then
    ! grep -Eq '[0-9]' "$artifacts/$tag-value.txt"
    # Isolated em-dash OCR is unstable (often read as 'a'). Require its actual
    # wide horizontal glyph at 3x scale; blanks, digits and cell borders fail.
    read -r dash_w dash_h < <(convert "$artifacts/$tag-value.png" -colorspace Gray -threshold 70% -trim -format '%w %h\n' info:)
    test "$dash_w" -ge 24
    test "$dash_w" -le 60
    test "$dash_h" -ge 1
    test "$dash_h" -le 9
    printf '%s em-dash glyph=%sx%s pass\n' "$tag" "$dash_w" "$dash_h" >> "$artifacts/unknown-values.txt"
  else
    grep -Eq "(^|[^0-9])$expected([^0-9]|$)" "$artifacts/$tag-value.txt"
  fi
}
selected_stored() {
  selection_count 1
  ocr
  name_left=$(awk -F '\t' '$12=="Name"{print int($7/3);exit}' "$artifacts/current.tsv")
  read -r x y < <(row '^stored.bin$')
  dark=$(convert "$artifacts/current.png" -crop "20x20+$((name_left-4))+$((y-10))" +repage -colorspace Gray -threshold 35% -format '%[fx:round((1-mean)*w*h)]' info:)
  test "$dark" -ge 40
}
resize 1440 900
open_archive packed.zip
wait_status '^3 entries'
select_row '^stored.bin$'
selection_count 1
menu_action Layout '^packed$'
assert_order $'folder\ndeflated.txt\nstored.bin' packed-ascending
selected_stored
packed_value stored.bin 4096 packed-stored
packed_value deflated.txt "$deflated_packed" packed-deflated
selection_count 1
capture_sizes packed-ascending
menu_action Layout '^packed$'
assert_order $'folder\nstored.bin\ndeflated.txt' packed-descending
selected_stored
capture_sizes packed-descending
pass packed-ascending-descending-directories-first-selection-retained
resize 640 600
ocr
locate '^Packed$' > /dev/null
! locate '^Size$' > /dev/null
! locate '^Type$' > /dev/null
screen 640x600-packed-column
menu_action Layout '^name$'
ocr
locate '^Size$' > /dev/null
! locate '^Packed$' > /dev/null
screen 640x600-size-column
menu_action Layout '^packed$'
ocr
locate '^Packed$' > /dev/null
! locate '^Size$' > /dev/null
screen 640x600-packed-restored
resize 1440 900
selected_stored
pass narrow-sort-switches-size-packed-columns
open_extract
choose_extract Selected Full
output_name "$outputs/selected"
wait_status 'Extracted 1 entries'
cmp "$run_dir/input/stored.bin" "$outputs/selected/stored.bin"
test "$(find "$outputs/selected" -mindepth 1 | wc -l)" -eq 1
selection_count 1
screen extracted-selected-after-packed-sort
pass packed-sort-selection-extracts-original-selected-file
for archive in solid.7z sample.tgz; do
  open_archive "$archive"
  menu_action Layout '^packed$'
  open_row '^docs$'
  open_row '^nested$'
  packed_value data.bin unknown "$archive-unknown-packed"
  # The visible explanatory text must describe unavailable data, not zero.
  ocr
  awk -F '\t' '$1==5{printf "%s ",$12}' "$artifacts/current.tsv" | grep -q 'unavailable'
  capture_sizes "$archive-unknown-packed"
  pass "$archive-unknown-packed-is-dash"
done
open_archive sample.tar
menu_action Layout '^packed$'
open_row '^docs$'
open_row '^nested$'
packed_value data.bin 512 tar-aligned-packed
capture_sizes tar-payload-aligned-packed
pass tar-payload-packed-512-aligned
printf 'retain sentinel\n' | cmp - "$outputs/sentinel.txt"
sha256sum -c "$artifacts/source-before.sha256"
if grep -Ei 'panic|critical|segmentation' "$artifacts/gtk.log"; then exit 1; fi
printf 'GUI_PACKED_SORT_COLUMNS_UNKNOWN_SELECTION_EXTRACTION_OK %s\n' "$run_dir"
