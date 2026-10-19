import math, sys
from PIL import Image, ImageDraw
out = sys.argv[1]
def star(size, fill=(255,214,40,255), outline=(230,140,0,255)):
    k = 4
    big = size*k
    im = Image.new("RGBA", (big,big), (0,0,0,0))
    d = ImageDraw.Draw(im)
    cx = cy = big/2
    R, r = big*0.48, big*0.20
    pts = []
    for i in range(10):
        a = -math.pi/2 + i*math.pi/5
        rad = R if i%2==0 else r
        pts.append((cx+rad*math.cos(a), cy+rad*math.sin(a)+big*0.03))
    d.polygon(pts, fill=fill, outline=outline, width=max(1,k*1))
    # reflet
    d.ellipse((cx-big*0.12, cy-big*0.12, cx-big*0.02, cy-big*0.02), fill=(255,250,200,255))
    return im.resize((size,size), Image.LANCZOS)
star(24).save(out + "/img/star.png")
# panier 48x20
k=4; W,H=48,20
im = Image.new("RGBA",(W*k,H*k),(0,0,0,0)); d=ImageDraw.Draw(im)
d.polygon([(0,4*k),(W*k-1,4*k),(W*k-6*k,H*k-1),(6*k,H*k-1)], fill=(176,104,48,255), outline=(110,60,20,255), width=k)
for x in range(10*k, W*k-6*k, 8*k):
    d.line([(x,5*k),(x-2*k,H*k-2*k)], fill=(120,70,30,255), width=k)
d.rounded_rectangle((0,0,W*k-1,6*k), radius=3*k, fill=(140,80,35,255))
im.resize((W,H),Image.LANCZOS).save(out+"/img/basket.png")
# vignette 104x136 (portrait, format des vignettes officielles)
t = Image.new("RGBA",(104,136),(20,24,70,255)); d=ImageDraw.Draw(t)
for (x,y) in [(10,10),(90,18),(52,8),(24,60),(84,70),(96,112),(6,104),(60,52),(12,126)]:
    d.point((x,y), fill=(255,255,255,255))
for (s,x,y) in [(40,10,14),(28,60,40),(22,18,74)]:
    st = star(s); t.alpha_composite(st,(x,y))
b = Image.open(out+"/img/basket.png").resize((72,30),Image.LANCZOS)
t.alpha_composite(b,(16,100))
t.save(out+"/img/thumbnail.png")
print("ok")
