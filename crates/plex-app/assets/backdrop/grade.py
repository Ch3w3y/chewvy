"""Regenerate aurora.jpg and aurora-soft.jpg from the Commons original.

Usage: python3 -I grade.py <Aurora_(140959963).jpeg> <out-dir>
Crop to 16:9, soft-light a violet-to-teal vertical gradient, darken slightly,
then blur: a light pass for full-screen sign-in views and a heavier, dimmer
pass for in-app backgrounds behind posters.
"""
import sys
from PIL import Image, ImageChops, ImageEnhance, ImageFilter

src, out = sys.argv[1], sys.argv[2]
im = Image.open(src).convert("RGB")
w, h = im.size
th = int(w * 9 / 16)
y = int((h - th) * 0.3)
im = im.crop((0, y, w, y + th)).resize((2048, 1152), Image.LANCZOS)
stops = [(0, (110, 60, 210)), (0.45, (150, 70, 200)), (1, (30, 60, 90))]
g = Image.new("RGB", (1, 256))
for yy in range(256):
    t = yy / 255
    for (a, ca), (b, cb) in zip(stops, stops[1:]):
        if a <= t <= b:
            k = (t - a) / (b - a)
            g.putpixel((0, yy), tuple(int(ca[i] * (1 - k) + cb[i] * k) for i in range(3)))
            break
im = ImageChops.soft_light(im, g.resize(im.size))
im = ImageEnhance.Color(ImageEnhance.Brightness(im).enhance(0.82)).enhance(1.12)
im.filter(ImageFilter.GaussianBlur(2)).save(f"{out}/aurora.jpg", quality=84, optimize=True, progressive=True)
soft = ImageEnhance.Brightness(im.filter(ImageFilter.GaussianBlur(9))).enhance(0.7)
soft.save(f"{out}/aurora-soft.jpg", quality=82, optimize=True, progressive=True)
