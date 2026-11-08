#!/usr/bin/env bash
# Small real GTK name-filter flow; ASCII queries, literal case-insensitive names.
set -euxo pipefail
if [[ ${1:-} != --session ]]; then
 root=$(cd "$(dirname "$0")/.." && pwd)
 binary=$(realpath "${1:?usage: smoke_linux_filter.sh BINARY ARTIFACT_DIRECTORY}")
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
mkdir -p "$artifacts" "$fixture/fs/child" "$outputs" "$run_dir/zip-expected"
cp "$root/tests/fixtures/sample.7z" "$fixture/sample.7z"
cp "$root/tests/fixtures/sample.tar" "$fixture/sample.tar"
printf 'alpha original\n' > "$fixture/fs/alpha.txt"
printf 'beta selected bytes\n' > "$fixture/fs/beta.txt"
printf 'outside retained\n' > "$fixture/fs/outside.txt"
printf 'child bytes\n' > "$fixture/fs/child/leaf.txt"
cp "$fixture/fs/beta.txt" "$run_dir/zip-expected/"
printf 'keep sentinel\n' > "$outputs/sentinel.txt"
find "$fixture" -type f -exec sha256sum '{}' + > "$artifacts/source-before.sha256"
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
resize 1440 900
for archive in sample.7z sample.tar; do
 7z x -y "-o$run_dir/ref-$archive" "$fixture/$archive" > "$artifacts/ref-$archive.log"
 mkdir -p "$run_dir/expected-$archive/docs"
 cp -a "$run_dir/ref-$archive/docs/nested" "$run_dir/expected-$archive/docs/"
 open_archive "$archive"
 select_row '^other.txt$'
 selection_count 1
 set_filter DoCs
 shown 1 2
 row '^docs$' > /dev/null
 ! row '^other.txt$' > /dev/null
 selection_count 1
 screen "$archive-realtime-filter"
 # No click between Enter and Ctrl+A: it must select rows, not filter text.
 xdotool key Return
 sleep .3
 xdotool key ctrl+a
 sleep .3
 selection_count 2
 menu_action Edit '^Invert$'
 selection_count 1
 clear_filter
 shown 2 2
 checkbox other.txt checked
 checkbox docs unchecked
 pass "$archive-hidden-selection-additive-select-invert-visible"
 set_filter DoCs
 xdotool key Return
 open_row '^docs$'
 shown 3 3
 row '^empty$' > /dev/null
 row '^nested$' > /dev/null
 set_filter NeStEd
 shown 1 3
 xdotool key Return
 selection_count 1
 open_extract
 choose_extract Displayed Full
 output_name "$outputs/$archive-displayed"
 if [[ "$archive" = sample.7z ]]; then count=2; else count=1; fi
 wait_status "Extracted $count entries"
 diff -r "$run_dir/expected-$archive" "$outputs/$archive-displayed"
 test ! -e "$outputs/$archive-displayed/other.txt"
 selection_count 1
 screen "$archive-displayed-output"
 pass "$archive-filtered-displayed-bytes-exclude-hidden-outside"
 menu_action Edit '^Clear$'
 selection_count 0
 clear_filter
 shown 3 3
 if [[ "$archive" = sample.7z ]]; then
  set_filter '*.txt'
  shown 0 3
  xdotool key Return
  output_state before-no-match
  no_match_blocked
  output_state after-no-match
  cmp "$artifacts/before-no-match.manifest" "$artifacts/after-no-match.manifest"
  pass literal-no-glob-no-match-displayed-disabled-no-fallback
  clear_filter
  # Short wildcard-mode extension; leave literal/folder mode restored below.
  menu_action Layout '^All$'
  shown 6 6
  select_row '^docs/nested/data.bin$'
  selection_count 1
  set_filter '*.txt'
  shown 0 6
  screen wildcard-literal-zero
  click_word '^Use$' 0 100
  shown 2 6
  selection_count 1
  screen wildcard-star-txt
  set_filter '[o]ther.?xt'
  shown 1 6
  row '^other.txt$' > /dev/null
  selection_count 1
  xdotool key Return
  open_extract
  choose_extract Displayed Full
  output_name "$outputs/wildcard-other"
  wait_status 'Extracted 1 entries'
  cmp "$run_dir/ref-sample.7z/other.txt" "$outputs/wildcard-other/other.txt"
  test "$(find "$outputs/wildcard-other" -mindepth 1 | wc -l)" -eq 1
  selection_count 1
  screen wildcard-displayed-other-bytes
  set_filter '[a-]'
  shown 0 6
  ocr
  awk -F '\t' '$1==5{printf "%s ",$12}' "$artifacts/current.tsv" | grep -q 'Invalid name mask'
  selection_count 1
  screen wildcard-invalid-mask
  click_word '^Use$' 0 100
  shown 0 6
  ocr
  ! locate '^Invalid$' 0 100 > /dev/null
  clear_filter
  shown 6 6
  selection_count 1
  checkbox 'docs/nested/data.bin' checked
  screen wildcard-off-clear-restored
  menu_action Edit '^Clear$'
  menu_action Layout '^All$'
  shown 3 3
  pass wildcard-star-class-invalid-literal-restore-hidden-selection-and-displayed-bytes
 fi
 set_filter nested
 xdotool key Return
 open_row '^nested$'
 shown 1 1
 row '^data.bin$' > /dev/null
 pass "$archive-navigation-clears-filter"
done
menu_action File '^folder'
picker=$(timeout 20 xdotool search --sync --onlyvisible --name '^Open folder$' | head -n1)
xdotool windowfocus --sync "$picker"
sleep .5
xdotool key ctrl+l
sleep .5
xdotool mousemove 360 23 click 1 key ctrl+a
xdotool type --clearmodifiers --delay 0 "$fixture/fs/"
sleep .5
xdotool key Return
sleep .7
if xdotool search --onlyvisible --name '^Open folder$' >/dev/null; then click_word '^Browse$' 0 150; fi
wait_status '^4 items'
select_row '^outside.txt$'
set_filter ALPHA
shown 1 4
xdotool key Return
sleep .3
xdotool key ctrl+a
sleep .3
selection_count 2
menu_action Edit '^Invert$'
selection_count 1
clear_filter
shown 4 4
checkbox outside.txt checked
checkbox alpha.txt unchecked
menu_action Edit '^Clear$'
set_filter BETA
shown 1 4
xdotool key Return
select_row '^beta.txt$'
selection_count 1
checkbox beta.txt checked
clear_filter
set_filter alpha
shown 1 4
xdotool key Return
selection_count 1
checkbox alpha.txt unchecked
screen fs-hidden-stable-beta-selection
menu_action File '^Create$'
modal_word '^Continue$'
save=$(timeout 20 xdotool search --sync --onlyvisible --name '^Create archive — choose a new filename$' | head -n1)
! xdotool search --onlyvisible --name '^Select files to archive$' >/dev/null
xdotool windowfocus --sync "$save"
sleep .5
xdotool mousemove 360 16 click 1 key ctrl+a
xdotool type --clearmodifiers --delay 0 "$outputs/selected-beta.zip"
sleep .5
xdotool key Return
wait_status 'Created ZIP: 1 entries'
7z x -y "-o$run_dir/zip-actual" "$outputs/selected-beta.zip" > "$artifacts/zip-extract.log"
diff -r "$run_dir/zip-expected" "$run_dir/zip-actual"
screen fs-selected-zip-success
pass filesystem-visible-stable-id-hidden-selection-zip-exact-bytes
sha256sum -c "$artifacts/source-before.sha256"
printf 'keep sentinel\n' | cmp - "$outputs/sentinel.txt"
if grep -Ei 'panic|critical|segmentation' "$artifacts/gtk.log"; then exit 1; fi
printf 'GUI_NAME_FILTER_VISIBLE_SELECTION_DISPLAYED_FS_ZIP_OK %s\n' "$run_dir"
