"""Synthesize mojibake fixture -- Type 0 font PDF without ToUnicode CMap.

Strategy:
1. reportlab 으로 Type 0 (CID) font 사용 한국어 PDF 합성 (정상 ToUnicode CMap 포함).
2. Generated PDF byte stream 에서 `/ToUnicode <ref>` 항목 + 해당 CMap stream 제거.

Usage:
  python3 tests/fixtures/_synth/mojibake.py \
      crates/kebab-parse-pdf/tests/fixtures/mojibake.pdf
"""
import sys, re
from pathlib import Path
from reportlab.lib.pagesizes import A4
from reportlab.lib.units import mm
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.pdfgen import canvas

# Noto CJK TTC uses PostScript outlines which reportlab does not support.
# Use DejaVu Sans TTF (always available on Ubuntu) instead -- the fixture's
# invariant is /ToUnicode CMap absent, not a specific script.
DEJAVU_TTF = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
FONT_NAME = "DejaVuSans"
pdfmetrics.registerFont(TTFont(FONT_NAME, DEJAVU_TTF))

dst = Path(sys.argv[1])

# Step 1: 정상 PDF 합성
c = canvas.Canvas(str(dst), pagesize=A4)
c.setFont(FONT_NAME, 12)
y = A4[1] - 30*mm
for line in ["Mojibake fixture (no ToUnicode CMap)", "Text extraction yields garbage \x00\x01\x02"]:
    c.drawString(30*mm, y, line)
    y -= 16

c.save()

# Step 2: ToUnicode CMap 제거 (best-effort byte-level rewrite)
data = dst.read_bytes()
# pattern: "/ToUnicode <objref>" -- referenced indirect object 의 stream 까지 제거
new_data = re.sub(rb"/ToUnicode\s+\d+\s+\d+\s+R\b", b"", data)

if new_data == data:
    print("WARNING: /ToUnicode reference not found -- Tier 1 failed, try Tier 2", file=sys.stderr)
    sys.exit(2)

dst.write_bytes(new_data)
print(f"wrote {dst} ({dst.stat().st_size} bytes, ToUnicode stripped)")
