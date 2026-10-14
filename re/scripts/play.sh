#!/bin/bash
# Runs the test copy windowed and plays a script of timed steps: "<seconds>:shot" or "<seconds>:key:<keysym>".
# Usage: play.sh <name> <step...> < popww.ini
P=/home/dante/projects/pop-ww
T="$P/re/Test Games/Prince of Persia - Warrior Within"
W=~/.config/heroic/tools/proton/Proton-GE-latest/files
name=$1; shift
mkdir -p $P/re/shots
cp $P/target/i686-pc-windows-gnu/release/popww.dll "$T/"
sed 's/$/\r/' > "$T/popww.ini"
rm -f "$T/PrinceSM" "$T/POPError.DAT"
cd "$T"
last=${@: -1}; last=${last%%:*}
WINEPREFIX=$P/re/prefix-vanilla/pfx WINEDEBUG=-all timeout $((last + 5)) $W/bin/wine PrinceOfPersia.exe > /dev/null 2>&1 &
pid=$!
prev=0
for step in "$@"; do
    t=${step%%:*}; action=${step#*:}
    sleep $((t - prev)); prev=$t
    id=$(xdotool search --name 'Prince of Persia' 2>/dev/null | tail -1)
    [ -z "$id" ] && continue
    case $action in
        shot) import -window "$id" -resize 640x $P/re/shots/$name-$t.png ;;
        key:*) xdotool windowactivate --sync "$id" keydown ${action#key:} sleep 0.15 keyup ${action#key:} ;;
    esac
done
wait $pid
WINEPREFIX=$P/re/prefix-vanilla/pfx $W/bin/wineserver -k 2>/dev/null
rm -f "$T/PrinceSM"
ls $P/re/shots/$name-*
