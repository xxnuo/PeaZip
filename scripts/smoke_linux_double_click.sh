#!/usr/bin/env bash
# Actual GTK name double-clicks; checkbox selection, history and source bytes.
set -euxo pipefail
if [[ ${1:-} != --session ]]; then
 root=$(cd "$(dirname "$0")/.." && pwd)
 binary=$(realpath "${1:?usage: smoke_linux_double_click.sh BINARY ARTIFACT_DIRECTORY}")
 mkdir -p "${2:?artifact directory required}"
 output=$(realpath "$2")
 runtime=$(mktemp -d)
 chmod 700 "$runtime"
 trap 'rm -rf "$runtime"' EXIT
 export XDG_RUNTIME_DIR="$runtime" GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_A11Y=none LANG=C.UTF-8 LC_ALL=C.UTF-8
 export OMP_THREAD_LIMIT=1
 xvfb-run -a -s '-screen 0 1440x900x24 -nolisten tcp' dbus-run-session -- bash "$0" --session "$binary" "$output" "$root"
 exit
fi
binary=$2 root=$4
run_dir=$(mktemp -d "$3/run-XXXXXX")
artifacts="$run_dir/artifacts" fixture="$run_dir/fixture"
mkdir -p "$artifacts" "$fixture/subdir/empty"
printf 'plain source\n' > "$fixture/plain.txt"
printf '\000\377binary\n' > "$fixture/subdir/child.bin"
for extension in 7z tar tar.gz; do cp "$root/tests/fixtures/sample.$extension" "$fixture/sample.$extension"; done
find "$fixture" -type f -exec sha256sum '{}' + | sort > "$artifacts/source-before.sha256"
find "$fixture" -printf '%y %P\n' | sort > "$artifacts/tree-before.txt"
"$binary" > "$artifacts/gtk.log" 2>&1 &
app_pid=$!
cleanup() {
 xdotool keyup ctrl shift alt super 2>/dev/null || true
 kill "$app_pid" 2>/dev/null || true
 wait "$app_pid" 2>/dev/null || true
}
trap cleanup EXIT
printf '%s\n' "$run_dir" > "$3/latest-run-path"
printf 'app_pid=%s display=%s\n' "$app_pid" "$DISPLAY" > "$artifacts/handles.txt"
xmodmap -pm > "$artifacts/xmodmap.txt"
cat > "$artifacts/x11-state.c" <<'C'
#include <X11/Xlib.h>
#include <stdio.h>
int main(void) {
 Display *d=XOpenDisplay(NULL); if(!d) return 1;
 Window root, child; int rx,ry,wx,wy; unsigned mask;
 XQueryPointer(d,DefaultRootWindow(d),&root,&child,&rx,&ry,&wx,&wy,&mask);
 printf("pointer_modifier_mask=0x%x\n",mask); XCloseDisplay(d); return 0;
}
C
cc "$artifacts/x11-state.c" -lX11 -o "$artifacts/x11-state"
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
 read -r sx sy < <(locate '^selected$' 0 100 340)
 convert "$artifacts/current.png" -crop "${width}x30+0+$((sy-10))" +repage -resize 300% "$artifacts/selection.png"
 tesseract "$artifacts/selection.png" "$artifacts/selection" --psm 6 2> "$artifacts/selection-ocr.log"
 local count=$1
 if [[ "$count" == 0 ]]; then count='[0O]{1,2}'; fi
 grep -Eq "(^| )$count selected" "$artifacts/selection.txt"
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
double_name() {
 local name=$1 modifier=${2:-}
 read -r x y < <(row "$name")
 # Only the name child handles activation; never use the indicator or chevron.
 xdotool mousemove "$x" "$y"
 if [[ -n "$modifier" ]]; then
  xdotool keydown "$modifier"
  sleep .1
  printf '%s %s ' "$name" "$modifier" >> "$artifacts/modifier-state.txt"
  "$artifacts/x11-state" >> "$artifacts/modifier-state.txt"
 fi
 xdotool click --repeat 2 --delay 100 1
 if [[ -n "$modifier" ]]; then xdotool keyup "$modifier"; fi
 sleep .5
}
indicator() {
 read -r x y < <(row "$1")
 ocr
 nx=$(awk -F '\t' '$12=="Name"{print int($7/3);exit}' "$artifacts/current.tsv")
 ix=$nx
 if (( nx > 100 )); then ix=$((nx+12)); fi
 xdotool mousemove "$ix" "$y" click 1
 sleep .4
}
checkbox() {
 local name=$1 expected=$2 tag=$3
 read -r x y < <(row "$name")
 nx=$(awk -F '\t' '$12=="Name"{print int($7/3);exit}' "$artifacts/current.tsv")
 convert "$artifacts/current.png" -crop "20x20+$((nx-4))+$((y-10))" +repage "$artifacts/$tag-checkbox.png"
 pixels=$(convert "$artifacts/$tag-checkbox.png" -colorspace Gray -threshold 35% -format '%[fx:round((1-mean)*w*h)]' info:)
 if [[ "$expected" == checked ]]; then test "$pixels" -ge 40; else test "$pixels" -le 4; fi
 printf '%s %s %s pixels=%s\n' "$tag" "$name" "$expected" "$pixels" >> "$artifacts/checkbox-results.txt"
}
name_keyboard() {
 local name=$1 all_count=$2 atomic_count=$3 tag=$4
 select_row "$name"
 selection_count 1
 checkbox "$name" checked "$tag-name"
 xdotool key space
 sleep .3
 selection_count 0
 checkbox "$name" unchecked "$tag-name-space"
 xdotool key space
 sleep .3
 selection_count 1
 checkbox "$name" checked "$tag-space-restored"
 xdotool key ctrl+a
 sleep .3
 selection_count "$all_count"
 xdotool key ctrl+i
 sleep .3
 selection_count 0
 xdotool key ctrl+i
 sleep .3
 selection_count "$atomic_count"
 checkbox "$name" checked "$tag-keyboard-invert"
 xdotool key ctrl+i
 sleep .3
 selection_count 0
 select_row "$name"
 selection_count 1
 select_row "$name"
 selection_count 0
 indicator "$name"
 selection_count 1
 xdotool key space
 sleep .3
 selection_count 0
 checkbox "$name" unchecked "$tag-indicator-space"
 pass "$tag-name-space-control-a-i-indicator-toggle"
}
source_unchanged() {
 sha256sum -c "$artifacts/source-before.sha256"
 find "$fixture" -printf '%y %P\n' | sort > "$artifacts/tree-after.txt"
 cmp "$artifacts/tree-before.txt" "$artifacts/tree-after.txt"
}
rows() {
 ocr
 read -r hx hy < <(locate '^Name$')
 for name in "$@"; do locate "$name" 0 "$((hy+12))" "$((height-45))" "$((width-220))" 1 > /dev/null; done
}
context_modal() {
 local name=$1 tag=$2
 read -r x y < <(row "$name")
 xdotool mousemove "$x" "$y" click 1 key ctrl+n
 sleep .08
 screen "$tag-modal-between-clicks"
 xdotool key Escape
 sleep .08
 xdotool mousemove "$x" "$y" click 1
 sleep .4
 convert "$artifacts/$tag-modal-between-clicks.png" -resize 300% "$artifacts/$tag-modal-ocr.png"
 tesseract "$artifacts/$tag-modal-ocr.png" "$artifacts/$tag-modal" --psm 11 2> "$artifacts/$tag-modal-ocr.log"
 grep -Eq 'Create archive' "$artifacts/$tag-modal.txt"
 grep -Eq 'Continue' "$artifacts/$tag-modal.txt"
 # Real pointer scenario; GTK's final click count is not instrumented here.
 pass "$tag-modal-between-pointer-clicks"
}
context_history() {
 local name=$1 tag=$2
 read -r x y < <(row "$name")
 xdotool mousemove "$x" "$y" click 1 key alt+Right
 sleep .08
 screen "$tag-history-away"
 xdotool key alt+Left
 sleep .08
 xdotool mousemove "$x" "$y" click 1
 sleep .4
 convert "$artifacts/$tag-history-away.png" -resize 300% "$artifacts/$tag-history-ocr.png"
 tesseract "$artifacts/$tag-history-ocr.png" "$artifacts/$tag-history" --psm 11 2> "$artifacts/$tag-history-ocr.log"
 grep -Eq '^nested$' "$artifacts/$tag-history.txt"
 pass "$tag-enter-back-between-pointer-clicks"
}
filesystem_root() {
 wait_status '^5 items'
 rows '^subdir$' '^plain.txt$' '^sample.7z$'
}
archive_root() {
 wait_status "^${archive_entries:-6} entries"
 rows '^docs$' '^other.txt$'
}
archive_docs() {
 rows '^empty$' '^nested$'
 ! row '^other.txt$' > /dev/null
}
no_extraction() {
 source_unchanged
 ! xdotool search --onlyvisible --name '^(Extract archive|Select destination|Select folder to extract)' >/dev/null
}
archive_error() {
 ocr
 tesseract "$artifacts/ocr.png" "$artifacts/archive-error" --psm 6 2> "$artifacts/archive-error-ocr.log"
 tr '\n' ' ' < "$artifacts/archive-error.txt" > "$artifacts/archive-error-line.txt"
 grep -Eq 'Opening files inside archives is not available yet' "$artifacts/archive-error-line.txt"
 grep -Eq 'Use Extract.*to save the file' "$artifacts/archive-error-line.txt"
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
filesystem_root
if [[ ${PEAZIP_DOUBLE_CLICK_FOCUS_ONLY:-0} == 1 ]]; then
 for dimensions in '1440 900' '1024 600'; do
  read -r w h <<< "$dimensions"
  resize "$w" "$h"
  tag="${w}x${h}-focus"
  name_keyboard '^plain.txt$' 5 5 "$tag-fs"
  double_name '^subdir$'
  wait_status '^2 items'
  row '^child.bin$' > /dev/null
  screen "$tag-fs-double-click"
  menu_action Browser '^Back$'
  filesystem_root
  selection_count 0
  double_name '^sample.7z$'
  archive_root
  name_keyboard '^other.txt$' 2 4 "$tag-archive"
  double_name '^docs$'
  archive_docs
  double_name '^nested$'
  row '^data.bin$' > /dev/null
  screen "$tag-archive-double-click"
  source_unchanged
  menu_action File '^Close$'
  filesystem_root
  selection_count 0
  pass "$tag-current-binary-directory-archive-double-click"
 done
 printf 'GUI_DOUBLE_CLICK_FOCUS_SPACE_CONTROL_A_I_DIRECTORY_ARCHIVE_OK %s\n' "$run_dir"
 exit
fi
for dimensions in '1440 900' '1024 600'; do
 read -r w h <<< "$dimensions"
 resize "$w" "$h"
 tag="${w}x${h}"
 if [[ ${PEAZIP_DOUBLE_CLICK_SKIP_WIDE_FS:-0} != 1 || "$width" != 1440 ]]; then
 selection_count 0
 name_keyboard '^plain.txt$' 5 5 "$tag-fs"
 context_modal '^subdir$' "$tag-fs"
 filesystem_root
 selection_count 0
 select_row '^plain.txt$'
 for modifier in ctrl shift alt Super_L; do
  double_name '^subdir$' "$modifier"
  wait_status '^2 items'
  row '^child.bin$' > /dev/null
  screen "$tag-fs-$modifier-entered"
  menu_action Browser '^Back$'
  filesystem_root
  selection_count 0
  checkbox '^plain.txt$' unchecked "$tag-fs-$modifier-navigation-clears-selection"
  select_row '^plain.txt$'
  selection_count 1
 done
 pass "$tag-fs-modified-double-click-opens-history-clears-selection"
 if [[ ${PEAZIP_DOUBLE_CLICK_MODIFIERS_ONLY:-0} == 1 ]]; then source_unchanged; exit; fi
 double_name '^subdir$'
 wait_status '^2 items'
 row '^child.bin$' > /dev/null
 screen "$tag-fs-directory-double-click"
 menu_action Browser '^Back$'
 filesystem_root
 menu_action Browser '^Forward$'
 wait_status '^2 items'
 row '^child.bin$' > /dev/null
 menu_action Browser '^Back$'
 filesystem_root
 pass "$tag-fs-directory-double-click-history"
 fi
 extensions=(7z)
 if (( width == 1440 )); then
  extensions+=(tar tar.gz)
  if [[ ${PEAZIP_DOUBLE_CLICK_SKIP_WIDE_7Z:-0} == 1 ]]; then extensions=(tar tar.gz); fi
 fi
 for extension in "${extensions[@]}"; do
  archive_entries=6
  if [[ "$extension" != 7z ]]; then archive_entries=5; fi
  double_name "^sample.$extension$"
  archive_root
  selection_count 0
  name_keyboard '^other.txt$' 2 4 "$tag-$extension"
  select_row '^other.txt$'
  selection_count 1
  for modifier in ctrl shift alt Super_L; do
   double_name '^docs$' "$modifier"
   archive_docs
   screen "$tag-$extension-$modifier-entered"
   menu_action Browser '^Back$'
   archive_root
   checkbox '^other.txt$' checked "$tag-$extension-$modifier-keeps-selection"
  done
  double_name '^docs$'
  archive_docs
  screen "$tag-$extension-archive-directory-double-click"
  double_name '^nested$'
  row '^data.bin$' > /dev/null
  menu_action Browser '^Back$'
  archive_docs
  menu_action Browser '^Back$'
  archive_root
  checkbox '^other.txt$' checked "$tag-$extension-history-keeps-selection"
  menu_action Browser '^Forward$'
  archive_docs
  menu_action Browser '^Forward$'
  row '^data.bin$' > /dev/null
  menu_action Browser '^Back$'
  menu_action Browser '^Back$'
  archive_root
  menu_action Edit '^Clear$'
  context_modal '^docs$' "$tag-$extension"
  archive_root
  selection_count 0
  context_history '^docs$' "$tag-$extension"
  archive_root
  selection_count 0
  # Seed a different checked row, then verify file activation keeps it checked.
  select_row '^docs$'
  double_name '^other.txt$'
  archive_error
  archive_root
  selection_count 2
  checkbox '^docs$' checked "$tag-$extension-file-error-docs"
  checkbox '^other.txt$' checked "$tag-$extension-file-error-file"
  no_extraction
  screen "$tag-$extension-file-error-no-extraction"
  double_name '^docs$'
  archive_docs
  ocr
  ! locate '^Opening$' >/dev/null
  menu_action Browser '^Back$'
  archive_root
  no_extraction
  pass "$tag-$extension-fs-open-archive-dirs-history-selection-file-error-recovery"
  # Reopen the same source after a first name click; new snapshot must work.
  select_row '^docs$'
  menu_action File '^Close$'
  filesystem_root
  double_name "^sample.$extension$"
  archive_root
  selection_count 0
  double_name '^docs$'
  archive_docs
  no_extraction
  pass "$tag-$extension-reopen-new-snapshot-double-click"
  menu_action File '^Close$'
  filesystem_root
  menu_action Edit '^Clear$'
  selection_count 0
 done
 source_unchanged
 screen "$tag-complete"
done
if grep -Ei 'panic|critical|segmentation' "$artifacts/gtk.log"; then exit 1; fi
pass sources-and-tree-unchanged-no-extraction
printf 'GUI_DOUBLE_CLICK_FS_ARCHIVE_HISTORY_SELECTION_MODIFIERS_ERROR_NARROW_OK %s\n' "$run_dir"
