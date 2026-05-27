"""Synthesize DCTDecode JPEG-wrapped PDF from PNG via reportlab drawImage.

reportlab's drawImage(<jpg_filename>) preserves JPEG bytes verbatim into the
PDF stream as /Filter /DCTDecode -- exactly what F1/F2 need.

Usage:
  python3 tests/fixtures/_synth/scanned_pdf.py \
      /build/cache/pdf-ocr-poc/images/page1-clean.png \
      crates/kebab-parse-pdf/tests/fixtures/scanned_page1.pdf
"""
import sys, tempfile, os
import reportlab.rl_config
reportlab.rl_config.useA85 = 0  # disable ASCII85 wrapper so image XObject uses /Filter /DCTDecode directly
from pathlib import Path
from PIL import Image
from reportlab.lib.pagesizes import A4
from reportlab.pdfgen import canvas

src = Path(sys.argv[1])
dst = Path(sys.argv[2])

# Step 1: PNG -> JPEG (quality 85, reproducible)
img = Image.open(src).convert("RGB")
with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tf:
    img.save(tf.name, "JPEG", quality=85)
    jpg_path = tf.name

# Step 2: reportlab canvas with drawImage(<jpg path>) -> DCTDecode passthrough
W, H = A4
c = canvas.Canvas(str(dst), pagesize=A4)
c.drawImage(jpg_path, 0, 0, width=W, height=H, preserveAspectRatio=True)
c.showPage()
c.save()

os.unlink(jpg_path)
print(f"wrote {dst} ({dst.stat().st_size} bytes)")
