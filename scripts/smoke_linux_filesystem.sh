#!/usr/bin/env bash
# Actual Linux GTK filesystem browser smoke; English OCR, native X11 pickers.
set -euxo pipefail
if [[ ${1:-} != --session ]]; then
  root=$(cd "$(dirname "$0")/.." && pwd)
  binary=$(realpath "${1:?usage: smoke_linux_filesystem.sh BINARY ARTIFACT_DIRECTORY}")
  mkdir -p "${2:?artifact directory required}"
  output=$(realpath "$2")
  runtime=$(mktemp -d)
  chmod 700 "$runtime"
  trap 'rm -rf "$runtime"' EXIT
  export XDG_RUNTIME_DIR="$runtime" GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_A11Y=none
  export LANG=C.UTF-8 LC_ALL=C.UTF-8
  xvfb-run -a -s '-screen 0 1440x900x24 -nolisten tcp' \
    dbus-run-session -- bash "$0" --session "$binary" "$output" "$root"
  exit
fi
binary=$2
root=$4
run_dir=$(mktemp -d "$3/run-XXXXXX")
artifacts="$run_dir/artifacts"
fixture="$run_dir/fixture"
mkdir -p "$artifacts" "$fixture/empty" "$fixture/subdir" "$fixture/中文目录" "$run_dir/outside" "$run_dir/zip-expected/empty"
printf 'alpha\nline-two\n' > "$fixture/alpha.txt"
printf '中文内容\n' > "$fixture/unicode-你好.txt"
printf 'original child\n' > "$fixture/subdir/child.txt"
printf 'not an archive\n' > "$fixture/bad.bin"
printf 'outside sentinel\n' > "$run_dir/outside/sentinel.txt"
cp "$root/tests/fixtures/sample.7z" "$fixture/sample.7z"
cp "$root/tests/fixtures/sample.tar" "$fixture/sample.tar"
cp "$root/tests/fixtures/sample.7z" "$fixture/subdir/held.7z"
cp "$root/tests/fixtures/sample.7z" "$run_dir/outside/outside.7z"
ln -s "$run_dir/outside/outside.7z" "$fixture/escape-link.7z"
cp "$fixture/alpha.txt" "$fixture/unicode-你好.txt" "$run_dir/zip-expected/"
"$binary" > "$artifacts/gtk.log" 2>&1 &
app_pid=$!
trap 'kill "$app_pid" 2>/dev/null || true; wait "$app_pid" 2>/dev/null || true' EXIT
printf '%s\n' "$run_dir" > "$3/latest-run-path"
printf 'app_pid=%s display=%s\n' "$app_pid" "$DISPLAY" > "$artifacts/handles.txt"
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
folder() {
  wait_status "^$1 items"
  row '^alpha.txt$' > /dev/null
  row '^sample.7z$' > /dev/null
}
go_folder() {
  xdotool key ctrl+l
  sleep .3
  xdotool type --clearmodifiers --delay 10 "$1"
  xdotool key Return
  sleep .4
}
open_selected() { menu_action File '^selected$'; }
archive_root() {
  wait_status '[0-9]+ entries'
  row '^docs$' > /dev/null
  row '^other.txt$' > /dev/null
}
pass() { printf '%s pass\n' "$1" | tee -a "$artifacts/results.txt"; }
resize 1440 900
menu_action File '^folder'
picker=$(timeout 20 xdotool search --sync --onlyvisible --name '^Open folder$' | head -n1)
xdotool windowfocus --sync "$picker"
sleep .5
screen 00-folder-dialog-before-location
xdotool key ctrl+l
sleep .5
xdotool mousemove 360 23 click 1 key ctrl+a
xdotool type --clearmodifiers --delay 0 "$fixture/"
sleep 1
screen 00-folder-dialog-path-entered
xdotool key Return
sleep 1
screen 01-open-folder-picker
# Native folder chooser's confirmation label is Browse, never Open archive.
if xdotool search --onlyvisible --name '^Open folder$' >/dev/null; then click_word '^Browse$' 0 150; fi
folder 9
selection_count 0
capture_sizes filesystem-root
pass native-folder-picker
select_row '^subdir$'
selection_count 1
open_selected
wait_status '^2 items'
row '^child.txt$' > /dev/null
menu_action Browser '^Back$'
folder 9
menu_action Browser '^Forward$'
wait_status '^2 items'
row '^held.7z$' > /dev/null
menu_action Browser '^Up$'
folder 9
pass folder-back-forward-up
# Typed absolute folder paths share the same browser history.
go_folder "$fixture/subdir"
wait_status '^2 items'
row '^child.txt$' > /dev/null
go_folder ..
folder 9
click_word '^Root$'
wait_status '[0-9]+ items'
row '^usr$' > /dev/null
screen 02-filesystem-root
menu_action Browser '^Back$'
folder 9
pass folder-path-and-root
select_row '^alpha.txt$'
select_row '^unicode-'
select_row '^empty$'
selection_count 3
screen 03-zip-selection
menu_action File '^Create$'
click_word '^Continue$'
save=$(timeout 20 xdotool search --sync --onlyvisible --name '^Create archive — choose a new filename$' | head -n1)
! xdotool search --onlyvisible --name '^Select files to archive$' >/dev/null
xdotool windowfocus --sync "$save"
sleep .5
screen 04-save-only-picker
xdotool mousemove 360 16 click 1 key ctrl+a
xdotool type --clearmodifiers --delay 0 "$fixture/created.zip"
sleep .5
xdotool key Return
wait_status 'Created ZIP: 3 entries'
test -f "$fixture/created.zip"
7z x -y "-o$run_dir/zip-actual" "$fixture/created.zip" > "$artifacts/zip-extract.log"
diff -r "$run_dir/zip-expected" "$run_dir/zip-actual"
7z l -slt "$fixture/created.zip" > "$artifacts/zip-list.txt"
screen 05-created-zip
pass selected-files-empty-directory-zip-bytes
menu_action File '^Close$'
folder 10
row '^created.zip$' > /dev/null
selection_count 0
pass close-created-zip-rescans-parent
select_row '^sample.7z$'
open_selected
archive_root
wait_status '^6 entries'
screen 06-7z-from-filesystem
open_row '^docs$'
row '^empty$' > /dev/null
row '^nested$' > /dev/null
menu_action Browser '^Back$'
archive_root
menu_action Browser '^Back$'
folder 10
menu_action Browser '^Forward$'
archive_root
menu_action Browser '^Forward$'
row '^nested$' > /dev/null
menu_action Browser '^Up$'
archive_root
menu_action Browser '^Up$'
folder 10
pass shared-filesystem-archive-history-and-up
select_row '^sample.tar$'
open_selected
archive_root
menu_action File '^Close$'
folder 10
pass tar-open-and-close-to-parent
select_row '^alpha.txt$'
selection_count 1
printf 'external addition\n' > "$fixture/new.txt"
xdotool key F5
folder 11
selection_count 0
row '^new.txt$' > /dev/null
screen 07-refreshed-folder
pass refresh-sees-addition-and-clears-selection
go_folder "$fixture/missing-folder"
wait_status 'Could not browse item'
row '^alpha.txt$' > /dev/null
row '^new.txt$' > /dev/null
screen 08-invalid-folder-preserved
go_folder "$fixture"
folder 11
pass invalid-folder-retains-prior-view
select_row '^bad.bin$'
open_selected
wait_status 'Could not browse item'
row '^alpha.txt$' > /dev/null
screen 09-bad-file-preserved
menu_action Edit '^Clear$'
select_row '^escape-link.7z$'
open_selected
wait_status 'Could not browse item: unsupported archive entry:.*escape-link'
row '^alpha.txt$' > /dev/null
printf 'outside sentinel\n' | cmp - "$run_dir/outside/sentinel.txt"
screen 10-link-rejected
pass invalid-archive-and-outside-link-rejected
menu_action Edit '^Clear$'
# Keep the displayed snapshot while replacing its entire logical root path.
mv "$fixture" "$run_dir/fixture-held"
mkdir -p "$fixture/subdir"
printf 'decoy\n' > "$fixture/subdir/decoy.txt"
printf 'not original archive\n' > "$fixture/subdir/held.7z"
open_row '^subdir$'
wait_status '^2 items'
row '^child.txt$' > /dev/null
! row '^decoy.txt$' > /dev/null
select_row '^held.7z$'
open_selected
archive_root
wait_status '^6 entries'
screen 11-held-root-original-archive
mv "$fixture" "$run_dir/fixture-decoy"
mv "$run_dir/fixture-held" "$fixture"
printf 'outside sentinel\n' | cmp - "$run_dir/outside/sentinel.txt"
pass held-root-snapshot-reads-original-after-whole-root-swap
sha256sum "$fixture/created.zip" "$fixture/alpha.txt" "$fixture/unicode-你好.txt" "$run_dir/outside/sentinel.txt" > "$artifacts/sha256.txt"
if grep -Ei 'panic|critical|segmentation|Could not create ZIP' "$artifacts/gtk.log"; then exit 1; fi
printf 'GUI_FILESYSTEM_NATIVE_NAV_CREATE_ZIP_HISTORY_REFRESH_SECURITY_OK %s\n' "$run_dir"
