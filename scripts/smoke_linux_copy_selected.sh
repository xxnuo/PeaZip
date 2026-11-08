#!/usr/bin/env bash
# Real GTK desktop Copy selected: native picker, bytes and no-overwrite.
set -euxo pipefail
if [[ ${1:-} != --session ]]; then
 root=$(cd "$(dirname "$0")/.." && pwd)
 binary=$(realpath "${1:?usage: smoke_linux_copy_selected.sh BINARY ARTIFACT_DIRECTORY}")
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
mkdir -p "$artifacts" "$fixture/tree/empty" "$outputs/existing" "$run_dir/outside"
printf 'first selected file\n' > "$fixture/alpha.txt"
printf '\000\377\001binary\n' > "$fixture/binary.bin"
printf 'Unicode tree payload\n' > "$fixture/tree/你好.txt"
printf 'outside sentinel\n' > "$run_dir/outside/sentinel.txt"
printf 'existing destination sentinel\n' > "$outputs/existing/keep.txt"
ln -s "$run_dir/outside" "$fixture/escape-link"
find "$fixture" "$outputs" "$run_dir/outside" -type f -exec sha256sum '{}' + > "$artifacts/source-before.sha256"
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
  read -r selected_x selected_y < <(locate '^selected$' 0 150 280)
  read -r word_left word_width < <(awk -F '\t' '$12=="selected" && $8/3>=150 && $8/3<=280 {x=int($7/3);w=int($9/3)} END{print x,w}' "$artifacts/current.tsv")
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
state() {
 { find "$fixture" "$outputs" "$run_dir/outside" -printf '%y %p %l\n' | sort; find "$fixture" "$outputs" "$run_dir/outside" -type f -exec sha256sum '{}' + | sort; } > "$artifacts/$1.manifest"
}
copy_picker() {
 menu_action Edit '^Copy$'
 picker=$(timeout 20 xdotool search --sync --onlyvisible --name '^Copy selected — choose a new folder name$' | head -n1)
 xdotool windowfocus --sync "$picker"
 sleep .5
}
output_name() {
 xdotool mousemove 360 16 click 1 key ctrl+a
 xdotool type --clearmodifiers --delay 0 "$1"
 sleep .4
 screen copy-picker
 xdotool key Return
}
wait_output() {
 for n in $(seq 1 30); do
  kill -0 "$app_pid"
  [[ -f "$1/tree/你好.txt" ]] && return
  sleep .3
 done
 return 1
}
error_text() {
 for n in $(seq 1 15); do
  screen current
  convert "$artifacts/current.png" -crop "${width}x40+0+$((height-40))" +repage -channel G -separate -resize 300% "$artifacts/error.png"
  tesseract "$artifacts/error.png" "$artifacts/error" --psm 6 2> "$artifacts/error-ocr.log"
  grep -Eqi "$1" "$artifacts/error.txt" && return
  sleep .2
 done
 cat "$artifacts/error.txt" >&2
 return 1
}
resize 1440 900
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
wait_status '^4 items'
open_row '^tree$'
wait_status '^2 items'
menu_action Browser '^Back$'
wait_status '^4 items'
select_row '^alpha.txt$'
select_row '^binary.bin$'
select_row '^tree$'
selection_count 3
state initial
click_word '^Copy$' 0 30 110
picker=$(timeout 20 xdotool search --sync --onlyvisible --name '^Copy selected — choose a new folder name$' | head -n1)
xdotool windowfocus --sync "$picker"
sleep .5
screen cancel-picker
xdotool key Escape
sleep .5
selection_count 3
state after-cancel
cmp "$artifacts/initial.manifest" "$artifacts/after-cancel.manifest"
pass cancel-preserves-selection-and-files
# A selected row hidden by the filter must still be included in the copy.
set_filter alpha
xdotool key Return
selection_count 3
screen hidden-selection
copy_picker
output_name "$outputs/copied"
wait_output "$outputs/copied"
for n in $(seq 1 20); do
 [[ -f "$outputs/copied/binary.bin" ]] && break
 sleep .2
done
mkdir -p "$run_dir/expected"
cp "$fixture/alpha.txt" "$fixture/binary.bin" "$run_dir/expected/"
cp -R "$fixture/tree" "$run_dir/expected/tree"
diff -r "$run_dir/expected" "$outputs/copied" > "$artifacts/copy-diff.txt"
selection_count 3
row '^alpha.txt$' > /dev/null
capture_sizes copied-selection-stays-in-source
pass hidden-selection-copy-bytes-empty-directory-and-no-navigation
state before-conflict
copy_picker
output_name "$outputs/existing/keep.txt"
# GTK confirmation belongs to our owned sentinel only. It is not a backend error.
for n in $(seq 1 20); do
 if ! xdotool search --onlyvisible --name '^Copy selected — choose a new folder name$' >/dev/null; then break; fi
 ocr
 if coords=$(locate '^Replace$'); then
  read -r x y <<< "$coords"
  xdotool mousemove "$x" "$y" click 1
 fi
 sleep .3
done
! xdotool search --onlyvisible --name '^Copy selected — choose a new folder name$' >/dev/null
error_text 'Could not copy selected items:.*(exists|exist|not empty)'
state after-conflict
cmp "$artifacts/before-conflict.manifest" "$artifacts/after-conflict.manifest"
screen existing-destination-refused
pass existing-destination-no-overwrite
# Copy must not consume the existing Forward route or navigate to its output.
menu_action Browser '^Forward$'
wait_status '^2 items'
row '^empty$' > /dev/null
menu_action Browser '^Back$'
wait_status '^4 items'
ocr
read -r clear_x clear_y < <(locate '^Clear$' 0 150 260)
xdotool mousemove "$clear_x" "$clear_y" click 1
selection_count 0
select_row '^escape-link$'
selection_count 1
state before-link
copy_picker
output_name "$outputs/link-copy"
error_text '(link|symbolic|unsupported|not supported)'
test ! -e "$outputs/link-copy"
state after-link
cmp "$artifacts/before-link.manifest" "$artifacts/after-link.manifest"
screen symlink-refused
pass symlink-refused-no-output
sha256sum -c "$artifacts/source-before.sha256"
if grep -Ei 'panic|critical|segmentation' "$artifacts/gtk.log"; then exit 1; fi
printf 'GUI_COPY_SELECTED_CANCEL_BYTES_HIDDEN_NO_OVERWRITE_LINK_OK %s\n' "$run_dir"
