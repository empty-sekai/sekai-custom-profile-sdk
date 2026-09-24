"""Builds ugui-fixture.otf, a tiny CFF-flavoured OpenType test font.

The glyphs are simple geometric outlines made for the uGUI text tests; the
font carries no third-party outlines. Run with fontTools installed:

    python make-ugui-fixture-font.py

The output is deterministic: timestamps are fixed.
"""

from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.t2CharStringPen import T2CharStringPen

UNITS_PER_EM = 1000
ASCENT = 880
DESCENT = -120
# 2026-01-01T00:00:00Z in seconds since 1904-01-01.
TIMESTAMP = 3850070400


def rectangle(pen, x0, y0, x1, y1):
    pen.moveTo((x0, y0))
    pen.lineTo((x0, y1))
    pen.lineTo((x1, y1))
    pen.lineTo((x1, y0))
    pen.closePath()


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


GLYPHS = [
    # name, advance, drawing, code points
    (".notdef", 500, None, []),
    ("space", 280, None, [0x20, 0xA0]),
    ("A", 613, draw_a, [0x41]),
    ("O", 745, draw_o, [0x4F]),
    ("kana", 1000, draw_kana, [0x3042]),
    ("bar", 1000, draw_bar, [0x30FC]),
    ("ring", 1000, draw_ring, [0x3002]),
    ("bracket", 1000, draw_bracket, [0x300C]),
]


def main():
    builder = FontBuilder(UNITS_PER_EM, isTTF=False)
    builder.setupGlyphOrder([name for name, *_ in GLYPHS])
    builder.setupCharacterMap(
        {code: name for name, _, _, codes in GLYPHS for code in codes}
    )
    charstrings = {}
    for name, advance, draw, _ in GLYPHS:
        pen = T2CharStringPen(advance, None)
        if draw is not None:
            draw(pen)
        charstrings[name] = pen.getCharString()
    builder.setupCFF(
        "UguiFixture-Regular",
        {"FullName": "UguiFixture Regular", "FamilyName": "UguiFixture"},
        charstrings,
        {},
    )
    metrics = {}
    for name, advance, draw, _ in GLYPHS:
        bounds = BoundsPen(None)
        if draw is not None:
            draw(bounds)
        metrics[name] = (advance, round(bounds.bounds[0]) if bounds.bounds else 0)
    builder.setupHorizontalMetrics(metrics)
    builder.setupHorizontalHeader(ascent=ASCENT, descent=DESCENT)
    builder.setupNameTable(
        {
            "familyName": "UguiFixture",
            "styleName": "Regular",
            "copyright": "Synthetic test font; no rights reserved.",
        }
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
    builder.save(Path(__file__).with_name("ugui-fixture.otf"))


if __name__ == "__main__":
    main()
