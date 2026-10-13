"""Builds the synthetic test fonts of the uGUI text tests.

- ugui-fixture.otf: a CFF-flavoured OpenType font, the text's own font.
- ugui-fixture-latin.ttf: a TrueType font with a few Latin glyphs, the first
  fallback face.
- ugui-fixture-cjk.ttc: a collection of four CFF-flavoured faces that draw
  the same characters differently, so the face picked from the collection is
  visible in the glyphs.

The glyphs are simple geometric outlines made for these tests; the fonts
carry no third-party outlines. Run with fontTools installed:

    python make-ugui-fixture-font.py

The output is deterministic: timestamps are fixed.
"""

from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.t2CharStringPen import T2CharStringPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib.ttCollection import TTCollection

UNITS_PER_EM = 1000
ASCENT = 880
DESCENT = -120
# 2026-01-01T00:00:00Z in seconds since 1904-01-01.
TIMESTAMP = 3850070400
COPYRIGHT = "Synthetic test font; no rights reserved."


def rectangle(pen, x0, y0, x1, y1):
    pen.moveTo((x0, y0))
    pen.lineTo((x0, y1))
    pen.lineTo((x1, y1))
    pen.lineTo((x1, y0))
    pen.closePath()


def rectangles(*boxes):
    def draw(pen):
        for box in boxes:
            rectangle(pen, *box)

    return draw


def ellipse(pen, cx, cy, rx, ry, clockwise):
    # Four cubic arcs; k is the usual circle approximation factor.
    k = 0.5523
    points = [
        ((cx + rx, cy), (cx + rx, cy + k * ry), (cx + k * rx, cy + ry), (cx, cy + ry)),
        ((cx, cy + ry), (cx - k * rx, cy + ry), (cx - rx, cy + k * ry), (cx - rx, cy)),
        ((cx - rx, cy), (cx - rx, cy - k * ry), (cx - k * rx, cy - ry), (cx, cy - ry)),
        ((cx, cy - ry), (cx + k * rx, cy - ry), (cx + rx, cy - k * ry), (cx + rx, cy)),
    ]
    if clockwise:
        points = [tuple(reversed(arc)) for arc in reversed(points)]
    pen.moveTo(tuple(round(value) for value in points[0][0]))
    for _, c1, c2, end in points:
        pen.curveTo(
            tuple(round(value) for value in c1),
            tuple(round(value) for value in c2),
            tuple(round(value) for value in end),
        )
    pen.closePath()


def draw_a(pen):
    # A triangle with a triangular counter.
    pen.moveTo((20, 0))
    pen.lineTo((306, 720))
    pen.lineTo((593, 0))
    pen.closePath()
    pen.moveTo((180, 90))
    pen.lineTo((433, 90))
    pen.lineTo((306, 420))
    pen.closePath()


def draw_o(pen):
    ellipse(pen, 372, 360, 330, 370, clockwise=True)
    ellipse(pen, 372, 360, 190, 250, clockwise=False)


def draw_kana(pen):
    # Two crossing bars and a bowl: enough strokes to fill an em square.
    rectangle(pen, 120, 560, 880, 640)
    rectangle(pen, 440, 40, 520, 840)
    ellipse(pen, 560, 300, 300, 230, clockwise=True)
    ellipse(pen, 560, 300, 220, 160, clockwise=False)


def draw_bar(pen):
    rectangle(pen, 80, 330, 920, 400)


def draw_ring(pen):
    ellipse(pen, 220, 110, 110, 110, clockwise=True)
    ellipse(pen, 220, 110, 60, 60, clockwise=False)


def draw_bracket(pen):
    pen.moveTo((560, 820))
    pen.lineTo((880, 820))
    pen.lineTo((880, 760))
    pen.lineTo((620, 760))
    pen.lineTo((620, 180))
    pen.lineTo((560, 180))
    pen.closePath()


# name, advance, drawing, code points
PRIMARY_GLYPHS = [
    (".notdef", 500, None, []),
    ("space", 280, None, [0x20, 0xA0]),
    ("A", 613, draw_a, [0x41]),
    ("O", 745, draw_o, [0x4F]),
    ("kana", 1000, draw_kana, [0x3042]),
    ("bar", 1000, draw_bar, [0x30FC]),
    ("ring", 1000, draw_ring, [0x3002]),
    ("bracket", 1000, draw_bracket, [0x300C]),
]

# The fallback faces lack U+005A, which no fixture font maps.
LATIN_GLYPHS = [
    (".notdef", 500, None, []),
    ("space", 250, None, [0x20]),
    # Never drawn: the text's own font maps U+0041.
    ("A", 600, rectangles((50, 0, 550, 700)), [0x41]),
    ("B", 587, rectangles(
        (90, 0, 190, 700),
        (190, 610, 450, 700),
        (190, 310, 480, 400),
        (190, 0, 500, 90),
        (410, 400, 480, 610),
        (430, 90, 500, 310),
    ), [0x42]),
]

# One entry per collection face: its style name and U+5173 as the face draws
# it (advance, outline).
CJK_FACES = [
    ("JP", 1000, rectangles((250, 0, 750, 800))),
    ("KR", 950, rectangles((100, -80, 900, 760))),
    ("SC", 1025, rectangles((60, 300, 940, 380), (460, -60, 540, 840))),
    ("TC", 900, rectangles((150, 100, 850, 700), (400, 700, 600, 860))),
]


def cjk_glyphs(advance, draw):
    return [
        (".notdef", 500, None, []),
        ("space", 333, None, [0x20]),
        # Never drawn: the Latin fallback maps U+0042 first.
        ("B", 1000, rectangles((0, 0, 1000, 1000)), [0x42]),
        ("guan", advance, draw, [0x5173]),
    ]


def left_side_bearings(glyphs):
    metrics = {}
    for name, advance, draw, _ in glyphs:
        bounds = BoundsPen(None)
        if draw is not None:
            draw(bounds)
        metrics[name] = (advance, round(bounds.bounds[0]) if bounds.bounds else 0)
    return metrics


def finish(builder, family, style, glyphs):
    builder.setupHorizontalMetrics(left_side_bearings(glyphs))
    builder.setupHorizontalHeader(ascent=ASCENT, descent=DESCENT)
    builder.setupNameTable(
        {"familyName": family, "styleName": style, "copyright": COPYRIGHT}
    )
    builder.setupOS2(
        sTypoAscender=ASCENT,
        sTypoDescender=DESCENT,
        usWinAscent=ASCENT,
        usWinDescent=-DESCENT,
    )
    builder.setupPost()
    head = builder.font["head"]
    head.created = TIMESTAMP
    head.modified = TIMESTAMP
    builder.font.recalcTimestamp = False
    return builder.font


def start(glyphs, is_ttf):
    builder = FontBuilder(UNITS_PER_EM, isTTF=is_ttf)
    builder.setupGlyphOrder([name for name, *_ in glyphs])
    builder.setupCharacterMap(
        {code: name for name, _, _, codes in glyphs for code in codes}
    )
    return builder


def cff_font(family, style, glyphs):
    builder = start(glyphs, is_ttf=False)
    charstrings = {}
    for name, advance, draw, _ in glyphs:
        pen = T2CharStringPen(advance, None)
        if draw is not None:
            draw(pen)
        charstrings[name] = pen.getCharString()
    postscript = f"{family.replace(' ', '')}-{style}"
    builder.setupCFF(
        postscript,
        {"FullName": f"{family} {style}", "FamilyName": family},
        charstrings,
        {},
    )
    return finish(builder, family, style, glyphs)


def ttf_font(family, style, glyphs):
    builder = start(glyphs, is_ttf=True)
    outlines = {}
    for name, _, draw, _ in glyphs:
        pen = TTGlyphPen(None)
        if draw is not None:
            draw(pen)
        outlines[name] = pen.glyph()
    builder.setupGlyf(outlines)
    return finish(builder, family, style, glyphs)


def main():
    here = Path(__file__).parent
    cff_font("UguiFixture", "Regular", PRIMARY_GLYPHS).save(here / "ugui-fixture.otf")
    ttf_font("UguiFixture Latin", "Regular", LATIN_GLYPHS).save(
        here / "ugui-fixture-latin.ttf"
    )
    collection = TTCollection()
    collection.fonts = [
        cff_font(f"UguiFixture CJK {script}", "Regular", cjk_glyphs(advance, draw))
        for script, advance, draw in CJK_FACES
    ]
    collection.save(here / "ugui-fixture-cjk.ttc")


if __name__ == "__main__":
    main()
