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
assert_displayed_default() {
  local count=$1
  ocr
  awk -F '\t' '$1==5 {printf "%s ",$12} END{print ""}' "$artifacts/current.tsv" > "$artifacts/displayed-context.txt"
  grep -Eq "0 selected[;,:]? +$count in displayed scope[;,:]? +6 archive entries" "$artifacts/displayed-context.txt"
  # Read the radio's filled centre relative to its OCR label, not fixed screen
  # coordinates. The adjacent card background also handles disabled opacity.
  coordinates=$(awk -F '\t' '$12=="Displayed" {print int($7/3),int(($8+$10/2)/3);exit}' "$artifacts/current.tsv")
  if [[ -z "$coordinates" ]]; then
    left=$((width/2-230))
    convert "$artifacts/current.png" -crop "460x$((height-125))+$left+75" +repage -colorspace Gray -threshold 90% -resize 300% "$artifacts/scope-contrast.png"
    tesseract "$artifacts/scope-contrast.png" "$artifacts/scope-contrast" --psm 11 tsv 2> "$artifacts/scope-contrast-ocr.log"
    coordinates=$(awk -F '\t' -v left="$left" '$12=="Displayed" {print left+int($7/3),75+int(($8+$10/2)/3);exit}' "$artifacts/scope-contrast.tsv")
  fi
  test -n "$coordinates"
  read -r text_left radio_y <<< "$coordinates"
  checked=$(convert "$artifacts/current.png" -crop "6x6+$((text_left-19))+$((radio_y-3))" +repage -colorspace Gray -format '%[fx:mean]' info:)
  background=$(convert "$artifacts/current.png" -crop "6x6+$((text_left-33))+$((radio_y-3))" +repage -colorspace Gray -format '%[fx:mean]' info:)
  awk -v a="$checked" -v b="$background" 'BEGIN {d=a-b; if(d<0)d=-d; exit !(d>0.08)}'
}
assert_empty_continue_blocked() {
  ocr
  read -r cx cy < <(locate '^Cancel$' "$((width/2-230))" 100)
  # Boost only the native app modal footer so disabled Continue text is still
  # found by OCR, then actually click it and require the modal to remain open.
  left=$((width/2-230)); top=$((cy-20))
  convert "$artifacts/current.png" -crop "460x44+$left+$top" +repage -colorspace Gray -threshold 90% -resize 300% "$artifacts/empty-footer.png"
  tesseract "$artifacts/empty-footer.png" "$artifacts/empty-footer" --psm 6 tsv 2> "$artifacts/empty-footer-ocr.log"
  read -r x y < <(awk -F '\t' -v left="$left" -v top="$top" '$12=="Continue" {print left+int(($7+$9/2)/3),top+int(($8+$10/2)/3);exit}' "$artifacts/empty-footer.tsv")
  xdotool mousemove "$x" "$y" click 1
  sleep .4
  ! xdotool search --onlyvisible --name '^Extract — choose new folder name$' > /dev/null
  assert_displayed_default 0
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
  # All cases must opt in explicitly now that no-selection defaults to Displayed.
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
# Enter docs with the prior root selection, then clear it using the native menu.
open_row '^docs$'
menu_action Edit '^Clear$'
selection_count 0
row '^empty$' > /dev/null
row '^nested$' > /dev/null
open_extract
assert_displayed_default 4
modal_word '^Full$'
screen displayed-docs-default
modal_word '^Continue$'
output_name "$outputs/displayed-docs"
wait_status 'Extracted 4 entries'
diff -r "$run_dir/expected-selected-full" "$outputs/displayed-docs"
test ! -e "$outputs/displayed-docs/other.txt"
selection_count 0
pass displayed-docs-default-descendants-with-prefix-and-no-other
open_row '^empty$'
selection_count 0
output_state before-empty-continue
open_extract
assert_displayed_default 0
screen displayed-empty-default-disabled
assert_empty_continue_blocked
output_state after-empty-continue
cmp "$artifacts/before-empty-continue.manifest" "$artifacts/after-empty-continue.manifest"
screen displayed-empty-continue-blocked
# Only this explicit All choice permits extraction from an empty displayed view.
choose_extract All Full
output_name "$outputs/empty-explicit-all"
wait_status 'Extracted 6 entries'
diff -r "$run_dir/reference" "$outputs/empty-explicit-all"
pass empty-displayed-blocks-continue-until-explicit-all
output_state before-empty-cancel
open_extract
assert_displayed_default 0
modal_word '^Cancel$'
ocr
! locate '^Scope$' "$((width/2-230))" 100 > /dev/null
! xdotool search --onlyvisible --name '^Extract — choose new folder name$' > /dev/null
output_state after-empty-cancel
cmp "$artifacts/before-empty-cancel.manifest" "$artifacts/after-empty-cancel.manifest"
pass empty-displayed-cancel-without-output
# Flat mode deliberately remains at docs/empty: displayed rows must ignore it.
menu_action Layout '^All$'
selection_count 0
open_extract
assert_displayed_default 6
grep -F 'All entries view' "$artifacts/displayed-context.txt"
modal_word '^Full$'
screen displayed-flat-default
modal_word '^Continue$'
output_name "$outputs/displayed-flat"
wait_status 'Extracted 6 entries'
diff -r "$run_dir/reference" "$outputs/displayed-flat"
pass flat-default-displayed-covers-entire-archive
menu_action Layout '^All$'
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
