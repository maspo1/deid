"""PDF 테스트 픽스처 생성기 (개발용, reportlab + Pillow 필요).

python playground/pdf/make_pdfs.py  → playground/pdf/*.pdf 생성
각 파일의 정답은 tests/pdf.rs 에 고정되어 있다.
"""
import io
import os
import sys

from PIL import Image, ImageDraw, ImageFont
from reportlab.lib.pagesizes import A4
from reportlab.lib.utils import ImageReader
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.pdfgen import canvas

OUT = os.path.dirname(os.path.abspath(__file__))
os.makedirs(OUT, exist_ok=True)
MALGUN = r"C:\Windows\Fonts\malgun.ttf"
pdfmetrics.registerFont(TTFont("Malgun", MALGUN))


def new(name, **info):
    c = canvas.Canvas(os.path.join(OUT, name), pagesize=A4, pageCompression=1)
    for k, v in info.items():
        getattr(c, "set" + k)(v)
    return c


# 1) 표준 폰트(WinAnsi) 영문 텍스트 + 메타데이터
c = new("simple.pdf", Author="kim@example.com", Title="report")
c.setFont("Helvetica", 12)
c.drawString(72, 760, "Contact: 010-1234-5678")
c.drawString(72, 740, "Server IP 192.168.77.1 / version 1.0.0.1")
c.drawString(72, 720, "Mail hong@test.co.kr")
c.drawString(72, 700, "Fake 999-1234-5678 and 300.1.2.3")
c.save()

# 2) 한글 서브셋 TrueType (Type0/Identity-H, ToUnicode)
c = new("korean.pdf")
c.setFont("Malgun", 12)
c.drawString(72, 760, "담당자 연락처 010-2468-1357")
c.drawString(72, 740, "주민번호 881111-1234567")
c.drawString(72, 720, "가짜 주민번호 881711-1234567, 가짜IP 127.2.3.981")
c.drawString(72, 700, "비밀번호: S3cure!Pass")
c.save()

# 3) 한 줄을 여러 조각으로 출력 (생성기마다 흔한 형태)
c = new("split.pdf")
c.setFont("Helvetica", 12)
x = 72
for piece in ["Tel ", "010-", "9999-", "8888"]:
    c.drawString(x, 760, piece)
    x += pdfmetrics.stringWidth(piece, "Helvetica", 12)
t = c.beginText(72, 740)
t.setFont("Helvetica", 12)
t.textOut("card ")
t.textOut("4111-1111-")
t.textOut("1111-1111")
c.drawText(t)
c.save()


# 4) 스캔 문서 (글자가 그림 안에 있음) — JPEG / 무손실 각각
def scan_image(text):
    img = Image.new("RGB", (900, 200), "white")
    d = ImageDraw.Draw(img)
    f = ImageFont.truetype(MALGUN, 40)
    d.text((30, 30), "고객 연락처", fill="black", font=f)
    d.text((30, 110), text, fill="black", font=f)
    return img


for name, text, fmt in [("scan_jpeg.pdf", "010-5555-6666", "JPEG"), ("scan_flate.pdf", "010-7777-2222", "PNG")]:
    buf = io.BytesIO()
    scan_image(text).save(buf, fmt, quality=95)
    buf.seek(0)
    c = new(name)
    c.drawImage(ImageReader(buf), 50, 500, width=450, height=100)
    c.save()

print("생성 완료:", sorted(os.listdir(OUT)))
