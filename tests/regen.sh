#!/bin/bash
# Regenerates the expected output of the test cases with the original
# asm68k, run under Wine.
#
#   ASM68K=/path/to/asm68k.exe tests/regen.sh [case ...]
#
# Each case is a source file in tests/cases. Its first line gives the
# switches to assemble it with. The object code, listing and messages that
# asm68k produces are kept next to it, for the tests to compare against.

set -e
: "${ASM68K:?set ASM68K to the path of asm68k.exe}"
ASM68K=$(realpath "$ASM68K")
cd "$(dirname "$0")/cases"
export WINEDEBUG=-all

cases=("$@")
[ ${#cases[@]} -eq 0 ] && cases=(*.68k)

for src in "${cases[@]}"; do
    name=$(basename "$src" .68k)
    switches=$(sed -n '1s/^; switches: //p' "$name.68k")
    rm -f "$name.bin" "$name.lst" "$name.msg"
    # Messages name the file by its full path in upper case; reduce that to
    # the name of the file as it was given.
    wine "$ASM68K" $switches "$name.68k,$name.bin,,$name.lst" 2>&1 | tr -d '\r' |
        grep -E '\([0-9]+\) : (Warning|Error|Fatal) : ' |
        sed -E 's/^.*[\\\/]([^\\\/]+\([0-9]+\) : )/\1/' |
        awk '{ n = index($0, "("); print tolower(substr($0, 1, n - 1)) substr($0, n) }' > "$name.msg" || true
    [ -s "$name.msg" ] || rm -f "$name.msg"
    # Without object code there were errors, and what the listing shows
    # for lines in error is of no interest.
    [ -f "$name.bin" ] || rm -f "$name.lst"
    echo "$name: $(ls "$name".bin "$name".lst "$name".msg 2>/dev/null | tr '\n' ' ')"
done
