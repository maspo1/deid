"""앱 아이콘(app.ico) 생성 스크립트 — 개발용 (Pillow 필요). exe 에는 결과 ico 만 포함된다.

디자인: 파란 바탕 위 문서 한 장, 문서의 한 줄이 마스킹 막대(***)로 가려져 있고 돋보기가 검사 중.
큰 캔버스(1024)에 그린 뒤 크기별로 축소해 작은 크기에서도 형태가 뭉개지지 않게 한다.
"""
from pathlib import Path
from PIL import Image, ImageDraw

S = 1024
OUT = Path(__file__).with_name("app.ico")


def draw(size_hint: int) -> Image.Image:
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    small = size_hint <= 24

    # 바탕: 둥근 사각형 (위→아래 그라데이션)
    bg = Image.new("RGBA", (S, S))
    gd = ImageDraw.Draw(bg)
    top, bot = (37, 99, 235), (30, 58, 138)
    for y in range(S):
        t = y / (S - 1)
        gd.line([(0, y), (S, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(top, bot)) + (255,))
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([24, 24, S - 24, S - 24], radius=200, fill=255)
    im.paste(bg, (0, 0), mask)

    # 문서
    x0, y0, x1, y1 = 230, 150, 730, 860
    fold = 140
    d.polygon([(x0, y0), (x1 - fold, y0), (x1, y0 + fold), (x1, y1), (x0, y1)], fill=(255, 255, 255, 255))
    d.polygon([(x1 - fold, y0), (x1 - fold, y0 + fold), (x1, y0 + fold)], fill=(191, 219, 254, 255))

    # 문서 줄 (작은 크기에서는 굵고 적게)
    lines = [330, 520, 710] if small else [300, 390, 570, 660, 750]
    lw = 56 if small else 34
    for y in lines:
        d.rounded_rectangle([x0 + 60, y, x1 - 80, y + lw], radius=lw // 2, fill=(148, 163, 184, 255))
    # 마스킹 막대 (***) — 비식별화의 상징
    by = 425 if small else 465
    bh = 90 if small else 76
    d.rounded_rectangle([x0 + 50, by, x1 - 50, by + bh], radius=18, fill=(15, 23, 42, 255))
    if not small:
        for i in range(3):
            cx, cy = x0 + 150 + i * 100, by + bh // 2
            for k in range(3):  # 별표: 선 3개
                import math
                a = math.pi / 3 * k
                dx, dy = 26 * math.cos(a), 26 * math.sin(a)
                d.line([(cx - dx, cy - dy), (cx + dx, cy + dy)], fill=(250, 204, 21, 255), width=14)

    # 돋보기
    cx, cy, r = 700, 690, 150
    ring = 48 if small else 40
    d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=(224, 242, 254, 140), outline=(250, 204, 21, 255), width=ring)
    import math
    a = math.radians(45)
    hx0, hy0 = cx + (r - 10) * math.cos(a), cy + (r - 10) * math.sin(a)
    hx1, hy1 = hx0 + 190 * math.cos(a), hy0 + 190 * math.sin(a)
    d.line([(hx0, hy0), (hx1, hy1)], fill=(250, 204, 21, 255), width=90 if small else 76)
    d.ellipse([hx1 - 45, hy1 - 45, hx1 + 45, hy1 + 45], fill=(250, 204, 21, 255))
    return im


sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
frames = [draw(s).resize((s, s), Image.LANCZOS) for s in sizes]
frames[-1].save(OUT, format="ICO", sizes=[(s, s) for s in sizes], append_images=frames[:-1])

print("written", OUT, OUT.stat().st_size)
