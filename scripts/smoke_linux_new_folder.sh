#!/usr/bin/env bash
# Real GTK desktop New folder: cancellation, validation, conflict and navigation.
set -euxo pipefail
if [[ ${1:-} != --session ]]; then
 root=$(cd "$(dirname "$0")/.." && pwd)
 binary=$(realpath "${1:?usage: smoke_linux_new_folder.sh BINARY ARTIFACT_DIRECTORY}")
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
mkdir -p "$artifacts" "$fixture/existing" "$outputs"
printf 'source bytes unchanged\n' > "$fixture/source.txt"
printf 'existing directory sentinel\n' > "$fixture/existing/keep.txt"
printf 'outside sentinel unchanged\n' > "$outputs/sentinel.txt"
find "$fixture" "$outputs" -type f -exec sha256sum '{}' + > "$artifacts/source-before.sha256"
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
state() { { find "$fixture" "$outputs" -printf '%y %p\n' | sort; find "$fixture" "$outputs" -type f -exec sha256sum '{}' + | sort; } > "$artifacts/$1.manifest"; }
modal() { ocr; locate '^Cancel$' "$((width/2-230))" 100 > /dev/null; }
modal_word() { click_word "$1" "$((width/2-230))" 100 "$((height-50))" "$((width/2+230))" 1; }
name_text() {
 # Locate the visible Folder name label; the entry is directly below it.
 ocr
 read -r nx ny < <(locate '^name$' "$((width/2-230))" 100 "$((height-50))" "$((width/2+230))" 1)
 xdotool mousemove "$((width/2))" "$((ny+26))" click 1 key ctrl+a
 xdotool type --clearmodifiers --delay 0 "$1"
 sleep .3
}
closed() {
 ocr
 ! locate '^Cancel$' "$((width/2-230))" 100 > /dev/null
}
create_disabled() {
 ocr
 read -r cx cy < <(locate '^Cancel$' "$((width/2-230))" 100)
 left=$((width/2-230)); top=$((cy-20))
 convert "$artifacts/current.png" -crop "460x44+$left+$top" +repage -colorspace Gray -threshold 90% -resize 300% "$artifacts/invalid-footer.png"
 tesseract "$artifacts/invalid-footer.png" "$artifacts/invalid-footer" --psm 6 tsv 2> "$artifacts/invalid-footer-ocr.log"
 read -r x y < <(awk -F '\t' -v x="$left" -v y="$top" 'tolower($12)=="create"{print x+int(($7+$9/2)/3),y+int(($8+$10/2)/3);exit}' "$artifacts/invalid-footer.tsv")
 xdotool mousemove "$x" "$y" click 1
 sleep .3
 modal
}
resize 1440 900
menu_action File '^folder'
picker=$(timeout 20 xdotool search --sync --onlyvisible --name '^Open folder$' | head -n1)
xdotool windowfocus --sync "$picker"
sleep .5
xdotool key ctrl+l
sleep .5
xdotool mousemove 360 23 click 1 key ctrl+a
xdotool type --clearmodifiers --delay 0 "$fixture/"
sleep .5
xdotool key Return
sleep .7
if xdotool search --onlyvisible --name '^Open folder$' >/dev/null; then click_word '^Browse$' 0 150; fi
wait_status '^2 items'
state initial
click_word '^New' 0 30 110
modal
name_text cancelled
screen cancel-dialog
modal_word '^Cancel$'
closed
state after-cancel
cmp "$artifacts/initial.manifest" "$artifacts/after-cancel.manifest"
pass toolbar-cancel-no-write
menu_action File '^New'
modal
name_text escaped
xdotool key Escape
sleep .4
closed
state after-escape
cmp "$artifacts/initial.manifest" "$artifacts/after-escape.manifest"
pass native-menu-escape-no-write
menu_action File '^New'
modal
name_text '../bad'
screen invalid-name-create-disabled
create_disabled
state after-invalid
cmp "$artifacts/initial.manifest" "$artifacts/after-invalid.manifest"
test ! -e "$run_dir/bad"
pass invalid-name-create-disabled-no-write
modal_word '^Cancel$'
set_filter source
xdotool key Return
select_row '^source.txt$'
selection_count 1
menu_action File '^New'
modal
name_text existing
modal_word '^Create$'
for n in $(seq 1 12); do
 ocr
 convert "$artifacts/current.png" -crop "460x400+$((width/2-230))+$((height/2-200))" +repage -channel G -separate -resize 300% "$artifacts/conflict-text.png"
 tesseract "$artifacts/conflict-text.png" "$artifacts/conflict-text" --psm 6 2> "$artifacts/conflict-ocr.log"
 grep -Eqi '(already exists|file exists)' "$artifacts/conflict-text.txt" && break
 sleep .2
done
grep -Eqi '(already exists|file exists)' "$artifacts/conflict-text.txt"
modal
screen existing-name-conflict
state after-conflict
cmp "$artifacts/initial.manifest" "$artifacts/after-conflict.manifest"
pass existing-directory-conflict-no-overwrite
name_text createdfolder
modal_word '^Create$'
wait_status '^Created folder: createdfolder'
selection_count 0
row '^createdfolder$' > /dev/null
row '^existing$' > /dev/null
row '^source.txt$' > /dev/null
screen created-folder-filter-cleared
test -d "$fixture/createdfolder"
test "$(find "$fixture/createdfolder" -mindepth 1 | wc -l)" -eq 0
pass ascii-retry-refreshes-and-clears-filter
open_row '^createdfolder$'
wait_status '^0 items'
screen new-folder-empty
menu_action Browser '^Back$'
wait_status '^3 items'
row '^createdfolder$' > /dev/null
menu_action Browser '^Forward$'
wait_status '^0 items'
menu_action Browser '^Up$'
wait_status '^3 items'
row '^source.txt$' > /dev/null
pass new-directory-open-back-forward-up
sha256sum -c "$artifacts/source-before.sha256"
if grep -Ei 'panic|critical|segmentation' "$artifacts/gtk.log"; then exit 1; fi
printf 'GUI_NEW_FOLDER_CANCEL_ESCAPE_INVALID_CONFLICT_RETRY_HISTORY_OK %s\n' "$run_dir"
