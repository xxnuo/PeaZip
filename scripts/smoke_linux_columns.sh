#!/usr/bin/env bash
# Real GTK shared column resizing and horizontal scrolling.
set -euxo pipefail
if [[ ${1:-} != --session ]]; then
 root=$(cd "$(dirname "$0")/.." && pwd)
 binary=$(realpath "${1:?usage: smoke_linux_columns.sh BINARY ARTIFACT_DIRECTORY}")
 mkdir -p "${2:?artifact directory required}"
 output=$(realpath "$2")
 runtime=$(mktemp -d); chmod 700 "$runtime"
 trap 'rm -rf "$runtime"' EXIT
 export XDG_RUNTIME_DIR="$runtime" GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_A11Y=none LANG=C.UTF-8 LC_ALL=C.UTF-8
 xvfb-run -a -s '-screen 0 1440x900x24 -nolisten tcp' dbus-run-session -- bash "$0" --session "$binary" "$output" "$root"
 exit
fi
binary=$2 root=$4
run_dir=$(mktemp -d "$3/run-XXXXXX")
artifacts="$run_dir/artifacts" fixture="$run_dir/fixture" outputs="$run_dir/output"
mkdir -p "$artifacts" "$fixture/长中文目录用于检查列宽和横向滚动/empty" "$outputs"
cp "$root/tests/fixtures/sample.7z" "$fixture/sample.7z"
printf 'long filename source bytes\n' > "$fixture/长中文目录用于检查列宽和横向滚动/长中文文件名用于确认完整显示与稳定选择.txt"
printf 'short bytes\n' > "$fixture/short.txt"
printf 'keep sentinel\n' > "$outputs/sentinel.txt"
find "$fixture" "$outputs" -type f -exec sha256sum '{}' + > "$artifacts/source.sha256"
7z x -y "-o$run_dir/reference" "$fixture/sample.7z" > "$artifacts/reference.log"
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
  read -r selected_x selected_y < <(locate '^selected$' 0 150 300)
  read -r word_left word_width < <(awk -F '\t' '$12=="selected" && $8/3>=150 && $8/3<=300 {x=int($7/3);w=int($9/3)} END{print x,w}' "$artifacts/current.tsv")
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
output_state() {
  { find "$outputs" -printf '%y %P\n' | sort; find "$outputs" -type f -exec sha256sum '{}' + | sort; } > "$artifacts/$1.manifest"
}
set_filter() {
 ocr
 if coords=$(locate '^Filter$' 0 100); then
  read -r x y <<< "$coords"
 else
  read -r x y < <(locate '^Use$' 0 100)
  x=$((x-75))
 fi
 xdotool mousemove "$x" "$y" click 1 key ctrl+a
 xdotool type --clearmodifiers --delay 0 "$1"
 sleep .4
}
clear_filter() { click_word '^filter$' 0 100; }
shown() {
 ocr
 read -r sx sy < <(locate '^shown$' 0 100)
 left=$(awk -F '\t' '$12=="shown"{print int($7/3);exit}' "$artifacts/current.tsv")
 convert "$artifacts/current.png" -crop "180x28+$((left-130))+$((sy-10))" +repage -resize 300% "$artifacts/shown.png"
 tesseract "$artifacts/shown.png" "$artifacts/shown" --psm 7 2> "$artifacts/shown-ocr.log"
 grep -Eq "of $2 (rows|items) shown" "$artifacts/shown.txt"
 # Fixture counts are single digits. OCR the number separately so a zero is
 # not inferred as the letter O from the neighbouring Clear filter caption.
 of_left=$(awk -F '\t' '$12=="of"{print int($7/3);exit}' "$artifacts/current.tsv")
 convert "$artifacts/current.png" -crop "13x24+$((of_left-16))+$((sy-10))" +repage -resize 500% "$artifacts/shown-count.png"
 tesseract "$artifacts/shown-count.png" "$artifacts/shown-count" --psm 10 -c tessedit_char_whitelist=0123456789 2> "$artifacts/shown-count-ocr.log"
 test "$(tr -d '[:space:]' < "$artifacts/shown-count.txt")" = "$1"
}
checkbox() {
 local name=$1 expected=$2
 ocr
 name_left=$(awk -F '\t' '$12=="Name"{print int($7/3);exit}' "$artifacts/current.tsv")
 read -r x y < <(row "^$name$")
 n=$(convert "$artifacts/current.png" -crop "20x20+$((name_left-4))+$((y-10))" +repage -colorspace Gray -threshold 35% -format '%[fx:round((1-mean)*w*h)]' info:)
 if [[ "$expected" = checked ]]; then test "$n" -ge 40; else test "$n" -le 4; fi
}
no_match_blocked() {
 open_extract
 ocr
 read -r cx cy < <(locate '^Cancel$' "$((width/2-230))" 100)
 left=$((width/2-230)); top=$((cy-20))
 convert "$artifacts/current.png" -crop "460x44+$left+$top" +repage -colorspace Gray -threshold 90% -resize 300% "$artifacts/no-match-footer.png"
 tesseract "$artifacts/no-match-footer.png" "$artifacts/no-match-footer" --psm 6 tsv 2> "$artifacts/no-match-ocr.log"
 read -r x y < <(awk -F '\t' -v x="$left" -v y="$top" '$12=="Continue"{print x+int(($7+$9/2)/3),y+int(($8+$10/2)/3);exit}' "$artifacts/no-match-footer.tsv")
 screen no-match-displayed-disabled
 xdotool mousemove "$x" "$y" click 1
 sleep .4
 ! xdotool search --onlyvisible --name '^Extract — choose new folder name$' >/dev/null
 ocr; locate '^Scope$' 0 100 > /dev/null
 modal_word '^Cancel$'
}
# OCR anchors the shared header; only the 9px trailing handle hit area is derived.
header_left() {
 ocr
 awk -F '\t' -v label="$1" 'index($12,label)>0 {print int($7/3),int(($8+$10/2)/3);exit}' "$artifacts/current.tsv"
}
type_position() { read -r type_x type_y < <(header_left Type); }
name_handle() {
 type_position
 handle_x=$((type_x-10)); handle_y=$type_y
}
drag_name() {
 name_handle
 xdotool mousemove "$handle_x" "$handle_y" mousedown 1
 for offset in 20 40 60 80 100 120; do
  xdotool mousemove "$((handle_x+offset))" "$handle_y"
  sleep .05
 done
 xdotool mouseup 1
 sleep .4
}
assert_delta() { test "$(( $1-$2 ))" -ge "$3"; test "$(( $1-$2 ))" -le "$4"; }
# Type and the first visible Folder cell must translate by the same scroll delta.
record_alignment() {
 ocr
 awk -F '\t' 'index($12,"Type")>0{hx=int($7/3)} $12=="Folder" && !found {rx=int($7/3);found=1} END{if(!found || !hx)exit 1;print hx,rx}' "$artifacts/current.tsv" > "$artifacts/$1-alignment.txt"
}
scroll_table() {
 # Drag the real bottom scrollbar; X11 synthetic horizontal wheel is not
 # consistently delivered to the enclosing scroller by nested GTK lists.
 local direction=$1 from to
 if [[ "$direction" = 7 ]]; then from=420; to=690; else from=690; to=600; fi
 xdotool mousemove "$from" "$((height-35))" mousedown 1
 xdotool mousemove --sync "$to" "$((height-35))"
 xdotool mouseup 1
 sleep .4
}
resize 1440 900
open_archive sample.7z
capture_sizes archive-default-columns
select_row '^docs$'
selection_count 1
name_handle
before=$type_x
drag_name
type_position
assert_delta "$type_x" "$before" 105 135
screen mouse-name-resized
# No clicks between these keys: the drag should have focused its Adjustable owner.
xdotool key Right
sleep .3
type_position
assert_delta "$type_x" "$before" 115 145
xdotool key shift+Left
sleep .3
type_position
assert_delta "$type_x" "$before" 65 95
xdotool key Home
sleep .3
type_position
assert_delta "$type_x" "$before" -3 3
pass mouse-drag-and-right-shift-left-home
# At 1024 the same explicit width overflows. Button7 is X11 horizontal scroll.
drag_name
resize 1024 600
scroll_table 7
record_alignment before-scroll
scroll_table 6
record_alignment after-scroll
read -r h0 r0 < "$artifacts/before-scroll-alignment.txt"
read -r h1 r1 < "$artifacts/after-scroll-alignment.txt"
test "$h1" -gt "$h0"
test "$(( (h1-h0)-(r1-r0) ))" -ge -3
test "$(( (h1-h0)-(r1-r0) ))" -le 3
selection_count 1
screen archive-horizontal-header-body-aligned
xdotool mousemove 600 "$((height-35))" mousedown 1 mousemove --sync 183 "$((height-35))" mouseup 1
sleep .3
resize 1440 900
selection_count 1
# Sort and filter must not discard the explicit Name width.
type_position
saved_type=$type_x
click_word '^Name$'
type_position
assert_delta "$type_x" "$saved_type" -3 3
set_filter docs
xdotool key Return
type_position
assert_delta "$type_x" "$saved_type" -3 3
selection_count 1
set_filter ''
xdotool key Return
click_word '^docs$' 0 260 400 170 1
wait_status '^6 entries'
capture_sizes archive-docs-resized-columns
menu_action Browser '^Up$'
selection_count 1
open_extract
choose_extract Selected Full
output_name "$outputs/extracted"
wait_status '^Extracted'
mkdir -p "$run_dir/expected"
cp -R "$run_dir/reference/docs" "$run_dir/expected/docs"
diff -r "$run_dir/expected" "$outputs/extracted"
pass resized-sorted-filtered-folder-open-selected-extraction-bytes
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
wait_status '^3 items'
# Name widths are session shared; compensate only for the archive's 180px sidebar.
type_position
assert_delta "$saved_type" "$type_x" 150 210
select_row '^short.txt$'
selection_count 1
capture_sizes filesystem-long-cjk-columns
resize 1440 900
ocr
read -r fx fy < <(awk -F '\t' '$12=="Folder" {print int($7/3),int(($8+$10/2)/3);exit}' "$artifacts/current.tsv")
read -r sx sy < <(header_left Size)
xdotool mousemove "$((sx+125))" "$fy" click 1
wait_status '^2 items'
capture_sizes filesystem-long-cjk-child
menu_action Browser '^Back$'
wait_status '^3 items'
resize 700 600
screen narrow-original-layout
row '^short.txt$' > /dev/null
resize 1440 900
sha256sum -c "$artifacts/source.sha256"
if grep -Ei 'panic|critical|segmentation' "$artifacts/gtk.log"; then exit 1; fi
printf 'GUI_COLUMNS_DRAG_KEYBOARD_SCROLL_SELECTION_EXTRACT_FS_OK %s\n' "$run_dir"
