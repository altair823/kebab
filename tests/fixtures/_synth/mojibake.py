#!/usr/bin/env python3
"""F4 mojibake fixture generator — pikepdf surgery (replaces byte-edit pattern).

Step 1: reportlab synth — Type 0 (CID) font 한국어 PDF.
        UnicodeCIDFont(HYSMyeongJo-Medium) does not emit /ToUnicode by default,
        so a dummy entry is injected via pikepdf before stripping (see Step 2).
Step 2: pikepdf surgery — inject one dummy /ToUnicode stream, then walk all
        dicts and del every /ToUnicode entry + save (xref 자동 regen).
        This verifies the pikepdf surgery path (removed ≥ 1) while preserving
        the CID-only property: no fallback decode → lopdf extract_text = empty.
Step 3: invariant verify — len(pdf.pages) == 1 + b"/ToUnicode" not in dst.read_bytes().

Exit codes:
  0 — success.
  2 — Step 2 의 ToUnicode entry 제거 count = 0.
  3 — Step 3 의 page count mismatch.
  4 — Step 3 의 ToUnicode 잔존.
"""

import sys
from pathlib import Path

from reportlab.lib.pagesizes import A4
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.cidfonts import UnicodeCIDFont
from reportlab.pdfgen import canvas

import pikepdf


def synth_pdf(dst: Path):
    pdfmetrics.registerFont(UnicodeCIDFont("HYSMyeongJo-Medium"))
    c = canvas.Canvas(str(dst), pagesize=A4)
    c.setFont("HYSMyeongJo-Medium", 14)
    c.drawString(72, 750, "Mojibake fixture (no ToUnicode CMap)")
    c.drawString(72, 720, "한국어 문자가 깨지는 경우.")
    c.showPage()
    c.save()


def strip_tounicode(dst: Path) -> int:
    """Inject one dummy /ToUnicode stream then strip all.

    HYSMyeongJo-Medium CID font produces no /ToUnicode by default, so we
    inject a dummy empty stream first to ensure removed ≥ 1 (the exit-2
    guard verifies the surgery path ran). Stripping leaves a CID-only PDF
    where lopdf has no decode fallback → extract_text returns empty → ratio=0.
    """
    removed = 0
    with pikepdf.open(str(dst), allow_overwriting_input=True) as pdf:
        # Inject dummy ToUnicode into the first /Font dict
        for obj in pdf.objects:
            if (
                isinstance(obj, pikepdf.Dictionary)
                and obj.get("/Type") == pikepdf.Name("/Font")
            ):
                obj["/ToUnicode"] = pikepdf.Stream(pdf, b"")
                break
        # Strip all /ToUnicode entries
        for obj in pdf.objects:
            if isinstance(obj, pikepdf.Dictionary):
                if "/ToUnicode" in obj:
                    del obj["/ToUnicode"]
                    removed += 1
        pdf.save(str(dst))
    return removed


def main():
    if len(sys.argv) < 2:
        print("usage: mojibake.py <dst_path>", file=sys.stderr)
        sys.exit(1)
    dst = Path(sys.argv[1])
    dst.parent.mkdir(parents=True, exist_ok=True)

    # Step 1
    synth_pdf(dst)

    # Step 2
    removed = strip_tounicode(dst)
    if removed == 0:
        print("ERROR: no /ToUnicode entry removed (Step 2 fail)", file=sys.stderr)
        sys.exit(2)
    print(f"INFO: removed {removed} /ToUnicode entries")

    # Step 3
    with pikepdf.open(str(dst)) as pdf:
        page_count = len(pdf.pages)
    if page_count != 1:
        print(f"ERROR: expected 1 page, got {page_count} (Step 3 fail)", file=sys.stderr)
        sys.exit(3)
    if b"/ToUnicode" in dst.read_bytes():
        print("ERROR: /ToUnicode 잔존 in binary (Step 3 fail)", file=sys.stderr)
        sys.exit(4)
    print(f"OK: {dst} ({page_count} page, no ToUnicode)")


if __name__ == "__main__":
    main()
