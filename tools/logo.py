"""Génère le logo de Takt : un signal d'horloge (le « takt », le cycle) sur un
carré arrondi dégradé. Sorties : SVG, PNG pour la fenêtre, ICO pour l'exécutable.

    python tools/logo.py
"""
from pathlib import Path
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "crates" / "takt-ide" / "assets"
S = 1024
RADIUS = 230
TOP = (0, 122, 255)      # bleu système Apple
BOTTOM = (0, 190, 214)   # cyan
WAVE = [(206, 624), (332, 624), (332, 400), (512, 400), (512, 624), (692, 624), (692, 400), (818, 400)]
STROKE = 66
TICKS = [332, 512, 692]
TICK_Y, TICK_R = 770, 24


def render(size: int) -> Image.Image:
    k = 4  # sur-échantillonnage pour l'anticrénelage
    n = size * k
    f = n / S
    grad = Image.new("RGB", (n, n))
    px = grad.load()
    for y in range(n):
        for x in range(n):
            t = (x + y) / (2 * n)
            px[x, y] = tuple(int(a + (b - a) * t) for a, b in zip(TOP, BOTTOM))
    mask = Image.new("L", (n, n), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, n - 1, n - 1], radius=RADIUS * f, fill=255)
    img = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    img.paste(grad, (0, 0), mask)
    d = ImageDraw.Draw(img)
    pts = [(x * f, y * f) for x, y in WAVE]
    d.line(pts, fill="white", width=int(STROKE * f), joint="curve")
    r = STROKE * f / 2
    for x, y in (pts[0], pts[-1]):
        d.ellipse([x - r, y - r, x + r, y + r], fill="white")
    for x in TICKS:
        cx, cy, rr = x * f, TICK_Y * f, TICK_R * f
        d.ellipse([cx - rr, cy - rr, cx + rr, cy + rr], fill=(255, 255, 255, 190))
    return img.resize((size, size), Image.LANCZOS)


def svg() -> str:
    pts = " ".join(f"{x},{y}" for x, y in WAVE)
    ticks = "".join(f'<circle cx="{x}" cy="{TICK_Y}" r="{TICK_R}" fill="#fff" fill-opacity=".75"/>' for x in TICKS)
    hexc = lambda c: "#%02x%02x%02x" % c
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {S} {S}" width="{S}" height="{S}">
  <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="{hexc(TOP)}"/><stop offset="1" stop-color="{hexc(BOTTOM)}"/>
  </linearGradient></defs>
  <rect width="{S}" height="{S}" rx="{RADIUS}" fill="url(#g)"/>
  <polyline points="{pts}" fill="none" stroke="#fff" stroke-width="{STROKE}" stroke-linecap="round" stroke-linejoin="round"/>
  {ticks}
</svg>
'''


if __name__ == "__main__":
    ASSETS.mkdir(parents=True, exist_ok=True)
    (ASSETS / "logo.svg").write_text(svg(), encoding="utf-8")
    big = render(256)
    big.save(ASSETS / "logo-256.png")
    big.save(ASSETS / "takt.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])

    print("logo généré dans", ASSETS)
