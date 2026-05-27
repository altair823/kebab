#!/usr/bin/env bash
# Synthesize F6 (FlateDecode) and F7 (CCITTFaxDecode) fixtures.
#
# F6: Pillow Image.save('.pdf', 'PDF') default = FlateDecode raw pixel.
# F7: Pillow bilevel TIFF (group4) + ghostscript pdfwrite.
#
# Usage (from repo root):
#   bash tests/fixtures/_synth/flate_ccittfax.sh
set -euo pipefail

FIXTURES="crates/kebab-parse-pdf/tests/fixtures"

# --- F6: FlateDecode raw pixel ---
# Pillow RGB->PDF uses DCTDecode by default; write a minimal PDF manually.
python3 -c "
import zlib

def make_flatedecode_pdf(dst_path, width=300, height=200):
    raw = b'\xff\xff\xff' * width * height
    compressed = zlib.compress(raw, level=6)
    content = f'q {width} 0 0 {height} 0 0 cm /Im Do Q'.encode()
    buf = b'%PDF-1.4\n'
    offsets = {}
    offsets[1] = len(buf)
    buf += b'1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n'
    offsets[2] = len(buf)
    buf += b'2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n'
    offsets[3] = len(buf)
    buf += (f'3 0 obj\n<< /Type /Page /Parent 2 0 R '
            f'/MediaBox [0 0 {width} {height}] '
            f'/Contents 4 0 R '
            f'/Resources << /XObject << /Im 5 0 R >> >> >>\nendobj\n').encode()
    offsets[4] = len(buf)
    buf += f'4 0 obj\n<< /Length {len(content)} >>\nstream\n'.encode()
    buf += content + b'\nendstream\nendobj\n'
    offsets[5] = len(buf)
    hdr = (f'5 0 obj\n<< /Type /XObject /Subtype /Image '
           f'/Width {width} /Height {height} '
           f'/ColorSpace /DeviceRGB /BitsPerComponent 8 '
           f'/Filter /FlateDecode '
           f'/Length {len(compressed)} >>\nstream\n').encode()
    buf += hdr + compressed + b'\nendstream\nendobj\n'
    xref_offset = len(buf)
    buf += b'xref\n0 6\n0000000000 65535 f \n'
    for i in range(1, 6):
        buf += f'{offsets[i]:010d} 00000 n \n'.encode()
    buf += (f'trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n').encode()
    with open(dst_path, 'wb') as f:
        f.write(buf)
    print(f'F6 wrote {dst_path} ({len(buf)} bytes)')

make_flatedecode_pdf('${FIXTURES}/flate_raw.pdf')
"

echo "F6 verify:"
python3 -c "
import re
data = open('${FIXTURES}/flate_raw.pdf', 'rb').read()
filters = re.findall(rb'/Filter\s*[\[/][^\]\n]{0,40}', data)
print('  Filters:', [f.decode(errors='replace') for f in filters])
"

# --- F7: CCITTFaxDecode bilevel ---
python3 -c "
from PIL import Image, ImageDraw, ImageFont
im = Image.new('1', (600, 800), 1)
draw = ImageDraw.Draw(im)
try:
    font = ImageFont.truetype('/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc', 16, index=1)
    draw.text((50, 50), 'test ccittfax', fill=0, font=font)
except Exception:
    draw.text((50, 50), 'test ccittfax', fill=0)
im.save('/tmp/ccitt.tif', 'TIFF', compression='group4')
print('TIFF wrote')
"

gs -dNOPAUSE -dBATCH -dQUIET \
   -sDEVICE=pdfwrite \
   -dCompressFonts=false \
   -dEncodeMonoImages=true \
   -dMonoImageFilter=/CCITTFaxEncode \
   "-sOutputFile=${FIXTURES}/ccitt.pdf" \
   /tmp/ccitt.tif

rm -f /tmp/ccitt.tif
echo "F7 wrote: ${FIXTURES}/ccitt.pdf ($(stat -c%s "${FIXTURES}/ccitt.pdf") bytes)"

echo "F7 verify:"
if grep -qc "/CCITTFax" "${FIXTURES}/ccitt.pdf" 2>/dev/null; then
    echo "  CCITTFax found (OK)"
else
    echo "  WARNING: CCITTFax not found -- ghostscript may have re-encoded"
    grep -ao "/Filter[ /][A-Za-z]*" "${FIXTURES}/ccitt.pdf" | sort -u || true
fi
