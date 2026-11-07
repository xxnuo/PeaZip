#!/usr/bin/env bash
# Real GTK/X11 smoke: English labels, GTK 4.14; OCR locates controls and rows.
# File chooser text fields use GTK's layout; this does not cover every portal.
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
  xvfb-run -a -s '-screen 0 1440x900x24 -nolisten tcp' \
    dbus-run-session -- bash "$0" --session "$binary" "$artifacts" "$fixture"
  exit
fi
binary=$2
fixture=$4
run_dir=$(mktemp -d "$3/run-XXXXXX")
artifacts="$run_dir/artifacts"
mkdir -p "$artifacts" "$run_dir/reference"
printf 'preserve-me\n' > "$run_dir/sentinel.txt"
"$binary" > "$artifacts/gtk.log" 2>&1 &
app_pid=$!
cleanup() { kill "$app_pid" 2>/dev/null || true; wait "$app_pid" 2>/dev/null || true; }
trap cleanup EXIT
printf 'app_pid=%s display=%s\n' "$app_pid" "$DISPLAY" > "$artifacts/handles.txt"
printf '%s\n' "$run_dir" > "$3/latest-run-path"
window=$(timeout 20 xdotool search --sync --onlyvisible --name '^PeaZip$' | head -n1)
width=1024 height=600
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
    ($12 == word || $12 == word "..." || $12 == word "…") && $7/3 >= minx && $8/3 >= miny && $8/3 <= maxy && $7/3 <= maxx {if(!found || !first || $8<top) {x=int(($7+$9/2)/3); y=int(($8+$10/2)/3); top=$8; found=1}}
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
  read -r x y < <(locate "$2" 0 "$((menu_y+12))" 420 "$((menu_x+300))" 1)
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
keyboard_select() {
  local expected=$1 label=$2
  xdotool key ctrl+a
  sleep .3
  selection_count "$expected"
  screen "keyboard-$label"
  printf '%s selected=%s pass\n' "$label" "$expected" >> "$artifacts/keyboard-focus-results.txt"
}
blank_content() {
  xdotool mousemove "$((width-80))" "$((height-80))" click 1
}
capture_native_menus() {
  for dimensions in '1024 600' '1440 900'; do
    read -r w h <<< "$dimensions"
    resize "$w" "$h"
    for menu in File Tools; do
      menu_open "$menu"
      screen "${w}x${h}-$menu-menu"
      xdotool key Escape
    done
  done
  resize 1024 600
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
  resize 1024 600
}
assert_root() {
  ocr
  locate docs 180 > /dev/null
  locate other.txt 180 > /dev/null
  test "$(awk -F '\t' '$7/3>=180 && ($12=="docs" || $12=="other.txt") {n++} END{print n+0}' "$artifacts/current.tsv")" = 2
  ! locate nested 180 > /dev/null
  locate Name 180 > /dev/null
  locate Type 180 > /dev/null
  locate Size 180 > /dev/null
  status_text
  grep -Eq '6 entries' "$artifacts/status.txt"
}
assert_docs() {
  ocr
  locate empty 180 > /dev/null
  locate nested 180 > /dev/null
  ! locate other.txt 180 > /dev/null
  test "$(awk -F '\t' '$7/3>=180 && ($12=="Folder" || $12=="TXT") {n++} END{print n+0}' "$artifacts/current.tsv")" = 3
}
resize 1024 600
capture_sizes empty
menu_action File Open
picker=$(timeout 20 xdotool search --sync --onlyvisible --name '^Open archive$' | head -n1)
for attempt in 1 2 3; do
  xdotool windowfocus --sync "$picker"
  sleep 1
  xdotool key ctrl+l
  sleep 1
  # GTK location entry spans the top of the native chooser.
  xdotool mousemove 360 23 click 1 key ctrl+a
  xdotool type --clearmodifiers --delay 40 "$(dirname "$fixture")/"
  sleep 1
  screen "02-location-$attempt"
  xdotool key Return
  sleep 2
  if xdotool search --onlyvisible --name '^Open archive$' >/dev/null; then
    ocr
    if coordinates=$(locate "$(basename "$fixture")" 140 40 280); then
      read -r x y <<< "$coordinates"
      xdotool mousemove "$x" "$y" click --repeat 2 --delay 150 1
    fi
  fi
  for n in $(seq 1 30); do
    xdotool search --onlyvisible --name '^Open archive$' >/dev/null || break
    sleep .1
  done
  xdotool search --onlyvisible --name '^Open archive$' >/dev/null || break
done
if xdotool search --onlyvisible --name '^Open archive$' >/dev/null; then exit 1; fi
sleep 1
assert_root
# These regressions deliberately do not click Name or refocus an input.
keyboard_select 2 after-open
menu_action Edit Clear
keyboard_select 2 after-clear
menu_action Edit Clear
blank_content
keyboard_select 2 after-clear-blank
menu_action Edit Clear
capture_sizes archive
capture_native_menus
menu_action Edit Select
selection_count 2
menu_action Edit Clear
selection_count 0
# Always-only native accelerators must work while the menubar owns focus.
menu_open Browser
xdotool key Escape
xdotool key ctrl+l
sleep .3
xdotool type --clearmodifiers nativefocusprobe
ocr
# OCR can include the input caret as a trailing vertical bar.
awk -F '\t' '$12 ~ /^nativefocusprobe[|]?$/ {found=1} END{exit !found}' "$artifacts/current.tsv"
screen 16-native-focus-path
xdotool key ctrl+a
xdotool type --clearmodifiers /
xdotool key Return
sleep .4
selection_count 0
xdotool key ctrl+a
sleep .4
selection_count 2
menu_action Edit Clear
selection_count 0
menu_action Layout All
ocr
test "$(awk -F '\t' -v minx="$((width/2))" '$7/3>=minx && ($12=="Folder" || $12=="TXT" || $12=="BIN") {n++} END{print n+0}' "$artifacts/current.tsv")" = 6
screen 10-all-entries
keyboard_select 6 after-flat
menu_action Edit Clear
menu_open Layout
screen 11-layout-checked
xdotool key Escape
menu_action Layout All
assert_root
keyboard_select 2 after-folder
menu_action Edit Clear
menu_action Layout size
blank_content
keyboard_select 2 after-sort-size-blank
menu_action Edit Clear
menu_action Layout name
keyboard_select 2 after-sort-name
menu_action Edit Clear
ocr
read -r docs_x docs_y < <(locate docs 180)
# Folder opener is a separate 36-pixel cell at the right edge of its row.
xdotool mousemove "$((width-18))" "$docs_y" click 1
sleep .5
assert_docs
keyboard_select 3 after-folder-open
menu_action Edit Clear
capture_sizes docs
menu_action Browser Back
assert_root
keyboard_select 2 after-menu-back
menu_action Edit Clear
xdotool key alt+Right
sleep .4
assert_docs
keyboard_select 3 after-alt-forward
menu_action Edit Clear
xdotool key alt+Left
sleep .4
assert_root
keyboard_select 2 after-alt-back
menu_action Edit Clear
xdotool key alt+Right
sleep .4
assert_docs
xdotool key alt+Up
sleep .4
assert_root
blank_content
keyboard_select 2 after-alt-up-blank
menu_action Edit Clear
# The archive path input is immediately before Go. Enter a nested directory.
ocr
read -r go_x go_y < <(locate Go)
xdotool mousemove "$((go_x-80))" "$go_y" click 1
xdotool type --clearmodifiers --delay 20 should-be-replaced
xdotool key ctrl+a
xdotool type --clearmodifiers --delay 30 docs/nested/
selection_count 0
xdotool key alt+Left
sleep .3
assert_root
selection_count 0
printf 'path-editing Ctrl+A/Alt+Left protected pass\n' >> "$artifacts/keyboard-focus-results.txt"
xdotool key Return
sleep .5
ocr
locate data.bin 180 > /dev/null
selection_count 0
screen 08-path-nested
keyboard_select 1 after-enter-nested
menu_action Edit Clear
menu_action Browser Up
assert_docs
blank_content
keyboard_select 3 after-menu-up-blank
menu_action Edit Clear
menu_action Browser Up
assert_root
screen 09-navigation-root
click_word docs 180
screen 04-selected
test ! -e "$run_dir/output"
menu_action Tools Test
for n in $(seq 1 20); do
  kill -0 "$app_pid"
  status_text
  if grep -Eq 'Tested 6 entries.*26 bytes.*Content checksums: 3/3 files' "$artifacts/status.txt"; then break; fi
  sleep .2
done
grep -Eq 'Tested 6 entries.*26 bytes.*Content checksums: 3/3 files' "$artifacts/status.txt"
cp "$artifacts/status.txt" "$artifacts/test-status.txt"
screen 07-tested
ocr
read -r selected_x selected_y < <(locate selected)
convert "$artifacts/current.png" -crop "${width}x30+0+$((selected_y-10))" +repage -resize 300% "$artifacts/test-selection.png"
tesseract "$artifacts/test-selection.png" "$artifacts/tested-screen" --psm 6 2> "$artifacts/selection-ocr.log"
grep -Eq '1 selected' "$artifacts/tested-screen.txt"
test ! -e "$run_dir/output"
printf 'preserve-me\n' | cmp - "$run_dir/sentinel.txt"
printf 'TEST_ARCHIVE_READ_ONLY_REPORT_OK\n' > "$artifacts/test-readonly.txt"
keyboard_select 2 after-test
menu_action Edit Clear
click_word docs 180
selection_count 1
mkdir "$run_dir/output"
cp "$run_dir/sentinel.txt" "$run_dir/output/sentinel.txt"
menu_action Tools Extract
click_word Selected "$((width/2-230))" 100
click_word Full "$((width/2-230))" 100
click_word Continue
output_picker=$(timeout 20 xdotool search --sync --onlyvisible --name '^Extract — choose new folder name$' | head -n1)
xdotool windowfocus --sync "$output_picker"
sleep 1
screen 05-output-picker
xdotool mousemove 360 16 click 1 key ctrl+a
xdotool type --clearmodifiers --delay 40 "$run_dir/output/Extracted"
sleep 1
click_word Extract 0 150
for n in $(seq 1 100); do [[ -f "$run_dir/output/Extracted/docs/你好.txt" ]] && break; kill -0 "$app_pid"; sleep .1; done
sleep .5
screen 06-extracted
7z x -y "-o$run_dir/reference" "$fixture" > "$artifacts/reference.log"
diff -r "$run_dir/reference/docs" "$run_dir/output/Extracted/docs"
test ! -e "$run_dir/output/Extracted/other.txt"
printf 'preserve-me\n' | cmp - "$run_dir/output/sentinel.txt"
test -d "$run_dir/output/Extracted/docs/empty"
sha256sum "$run_dir/output/Extracted/docs/nested/data.bin" "$run_dir/output/Extracted/docs/你好.txt" > "$artifacts/sha256.txt"
menu_action Help About
ocr
locate Rust > /dev/null
screen 15-about-modal
capture_sizes about-modal
# About gives its Close action keyboard focus (white-on-dark text confuses OCR).
xdotool key Return
sleep .4
ocr
! locate Rust > /dev/null
keyboard_select 2 after-about
menu_action File Close
status_text
grep -Eq 'Open an archive to view its contents' "$artifacts/status.txt"
ocr
! locate docs 180 > /dev/null
! locate other.txt 180 > /dev/null
selection_count 0
screen 12-closed
menu_open Tools
screen 13-tools-disabled-after-close
xdotool key Escape
# Open must also work directly after native menu dismissal.
xdotool key ctrl+o
picker=$(timeout 20 xdotool search --sync --onlyvisible --name '^Open archive$' | head -n1)
screen 14-shortcut-open-picker
xdotool key Escape
for n in $(seq 1 30); do
  xdotool search --onlyvisible --name '^Open archive$' >/dev/null || break
  sleep .1
done
! xdotool search --onlyvisible --name '^Open archive$' >/dev/null
kill -0 "$app_pid"
if grep -Ei 'panic|critical|segmentation|could not open archive|could not extract archive|could not test archive' "$artifacts/gtk.log"; then exit 1; fi
printf 'GUI_CONTENT_FOCUS_REGRESSIONS_OK\n'
printf 'GUI_NATIVE_MENUS_SHORTCUTS_NAVIGATION_TEST_EXTRACT_CLOSE_OK %s\n' "$run_dir"
