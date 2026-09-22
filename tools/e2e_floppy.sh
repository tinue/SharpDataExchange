#!/bin/sh
# End-to-end check of sde's floppy support against the real PC-1600 ROM running in
# Calc-U-1600's headless harness. Manual (needs a sibling Calc-U-1600 checkout with its
# ROMs, and a build of headless/pc1600_cli with --save-dir); not run in CI.
#
#   tools/e2e_floppy.sh [path/to/Calc-U-1600]
#
# 1. sde puts BASIC (tokenized and ASCII), UTF-8 text and machine code on a fresh disk.
# 2. The ROM LOADs / INPUT#s / BLOADs them and SAVEs / PRINT#s / BSAVEs copies, KILLs a
#    file and SAVEs another; the disk is saved back.
# 3. sde reads the ROM's copies and compares them with what it wrote: this proves the ROM
#    accepts sde's files, and shows what the ROM itself writes (timestamps, attributes,
#    ASCII end-of-file, BSAVE headers, slot reuse).
set -eu

SDE_DIR=$(cd "$(dirname "$0")/.." && pwd)
CALC=$(cd "${1:-$SDE_DIR/../Calc-U-1600}" && pwd)
SDE="$SDE_DIR/target/debug/sde"
CLI="$CALC/headless/pc1600_cli"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/sde-e2e.XXXXXX")
echo "work dir: $WORK"

(cd "$SDE_DIR" && cargo build -q)
[ -x "$CLI" ] || (cd "$CALC" && tools/build_pc1600_cli.sh)

cd "$WORK"
sed 's/^disk-name: .*/disk-name: "SdeE2E"/' "$CALC/Qt6/resources/cards/formatted.floppy.yaml" > in.floppy.yaml

printf '10 PRINT "HALLO"\n20 A=1:GOTO 40\n30 END\n40 PRINT A\n' > hallo.bas
printf 'Gr\303\274\303\237e\nLINE 2\n' > note.txt
printf '\311\000\021\042' > mc.bin                    # C9 (RET) 00 11 22

"$SDE" put hallo.bas note.txt in.floppy.yaml:A:
"$SDE" put hallo.bas in.floppy.yaml:A:HALLOA.BAS -f ascii
"$SDE" put mc.bin in.floppy.yaml:A: --start-address C0C5
"$SDE" dir in.floppy.yaml:A

cat > probe.pc1600 <<'EOF'
model: PC-1600
plotter: ce1600p
floppy: SdeE2E

keys:
  - key: mode
  - type: NEW0
  - type: NEW "S0:",&1C5
  - key: mode
  - type: TIME=092213.4556
  - type: LOAD"X:HALLO.BAS"
  - wait:
  - type: SAVE"X:HALLO2.BAS"
  - wait:
  - type: LOAD"X:HALLOA.BAS"
  - wait:
  - type: SAVE"X:HALLOA2.BAS"
  - wait:
  - type: SAVE"X:ROMA.BAS",A
  - wait:
  - type: MAXFILES=2
  - type: OPEN"X:NOTE.TXT" FOR INPUT AS #1
  - wait:
  - type: INPUT#1,A$,B$
  - type: CLOSE #1
  - wait:
  - type: OPEN"X:NOTE2.TXT" FOR OUTPUT AS #2
  - wait:
  - type: PRINT#2,A$
  - type: PRINT#2,B$
  - type: CLOSE #2
  - wait:
  - type: BLOAD"X:MC.BIN"
  - wait:
  - type: BSAVE"X:MC2.BIN",#0,&C0C5,&C0C8
  - wait:
  - type: BSAVE"X:MC3.BIN",#0,&C0C5,&C0C8,&C0C5
  - wait:
  - type: KILL"X:HALLO.BAS"
  - wait:
  - type: SAVE"X:AFTER.BAS"
  - wait:
  - type: CLS
  - type: FILES"X:"
  - wait: 2
  - screenshot: files.png
  - type: CLS
  - type: PRINT DSKF"X:"
  - wait: 1
  - screenshot: dskf.png
  - saveas: floppy:SdeE2EOut
EOF

(cd "$CALC" && "$CLI" --preset "$WORK/probe.pc1600" --modules-dir "$WORK" --save-dir "$WORK") > run.log 2>&1 \
  || { cat run.log; exit 1; }
mv "$CALC"/files.png "$CALC"/dskf.png "$WORK"/ 2>/dev/null || true

OUT=SdeE2EOut.floppy.yaml
"$SDE" dir $OUT:A
mkdir -p out
"$SDE" get "$OUT:A:*" out --raw -q
fail=0
check() { if "$@"; then echo "ok:   $*"; else echo "FAIL: $*"; fail=1; fi; }

# The ROM re-saved sde's tokenized BASIC byte-identically (header included).
"$SDE" get in.floppy.yaml:A:HALLO.BAS sde_hallo.bin -f binary -q
check cmp -s sde_hallo.bin out/HALLO2.BAS
# LOADing sde's ASCII listing gives the same program.
check cmp -s sde_hallo.bin out/HALLOA2.BAS
# The ROM's own ASCII save has sde's ASCII format: CRLF line ends, a final 1A counted in
# the size. (Its listing text differs: LIST puts a space after a keyword such as END.)
check test "$(tail -c 3 out/ROMA.BAS | xxd -p)" = 0d0a1a
"$SDE" get in.floppy.yaml:A:HALLOA.BAS sde_halloa.raw --raw -q
check test "$(tail -c 3 sde_halloa.raw | xxd -p)" = 0d0a1a
# Text read by INPUT# and written back by PRINT# is the text sde wrote.
"$SDE" get in.floppy.yaml:A:NOTE.TXT sde_note.raw --raw -q
check cmp -s sde_note.raw out/NOTE2.TXT
# BLOAD put sde's bytes at C0C5; BSAVE of that range stores the same payload.
tail -c +17 out/MC2.BIN > mc2.payload
check cmp -s mc.bin mc2.payload
echo "--- sde's machine header vs BSAVE's (no auto-start), then BSAVE with auto-start:"
"$SDE" get in.floppy.yaml:A:MC.BIN sde_mc.raw --raw -q
xxd -l 16 sde_mc.raw
xxd -l 16 out/MC2.BIN
xxd -l 16 out/MC3.BIN
echo "--- ROM ASCII files, raw:"
xxd out/ROMA.BAS | tail -2
xxd out/NOTE2.TXT
echo "--- directory entries as the ROM wrote them:"
"$SDE" dir $OUT:A
echo "screenshots: $WORK/files.png $WORK/dskf.png"
[ $fail = 0 ] && echo "E2E OK" || { echo "E2E FAILED"; exit 1; }
