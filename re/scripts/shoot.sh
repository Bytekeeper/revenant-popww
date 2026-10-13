#!/bin/bash
# Runs the test copy windowed with the given popww.ini and screenshots the game window.
# Usage: shoot.sh <name> <seconds...> < popww.ini
P=/home/dante/projects/pop-ww
T="$P/re/Test Games/Prince of Persia - Warrior Within"
W=~/.config/heroic/tools/proton/Proton-GE-latest/files
name=$1; shift
mkdir -p $P/re/shots
cp $P/target/i686-pc-windows-gnu/release/popww.dll "$T/"
sed 's/$/\r/' > "$T/popww.ini"
rm -f "$T/PrinceSM" "$T/POPError.DAT"
cd "$T"
last=${@: -1}
WINEPREFIX=$P/re/prefix-vanilla/pfx WINEDEBUG=-all timeout $((last + 5)) $W/bin/wine PrinceOfPersia.exe > /dev/null 2>&1 &
pid=$!
prev=0
for t in "$@"; do
    sleep $((t - prev)); prev=$t
    id=$(xdotool search --name 'Prince of Persia' 2>/dev/null | tail -1)
    [ -n "$id" ] && import -window "$id" -resize 640x $P/re/shots/$name-$t.png
done
wait $pid
WINEPREFIX=$P/re/prefix-vanilla/pfx $W/bin/wineserver -k 2>/dev/null
rm -f "$T/PrinceSM"
ls $P/re/shots/$name-*
