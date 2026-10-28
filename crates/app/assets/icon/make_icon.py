"""Draws the RWord app icon. Run from this folder: python3 make_icon.py (needs Pillow)."""
from PIL import Image, ImageDraw, ImageFont, ImageFilter
K=4; S=1024*K
def s(v): return int(v*K)
def lerp(a,b,t): return tuple(int(a[i]+(b[i]-a[i])*t) for i in range(3))
img=Image.new("RGBA",(S,S),(0,0,0,0))
grad=Image.new("RGBA",(S,S)); gd=ImageDraw.Draw(grad)
top=(56,126,236); bot=(16,70,168)
for y in range(S): gd.line([(0,y),(S,y)],fill=lerp(top,bot,y/S)+(255,))
mask=Image.new("L",(S,S),0); ImageDraw.Draw(mask).rounded_rectangle([s(64),s(64),s(960),s(960)],radius=s(200),fill=255)
img.paste(grad,(0,0),mask)
L,T,R,B=176,168,848,872
sh=Image.new("RGBA",(S,S),(0,0,0,0)); ImageDraw.Draw(sh).rounded_rectangle([s(L+10),s(T+22),s(R+10),s(B+22)],radius=s(48),fill=(0,20,60,120))
sh=sh.filter(ImageFilter.GaussianBlur(s(18))); img.alpha_composite(sh)
d=ImageDraw.Draw(img)
d.rounded_rectangle([s(L),s(T),s(R),s(B)],radius=s(48),fill=(255,255,255,255))
d.rounded_rectangle([s(L),s(T),s(R),s(T+160)],radius=s(48),fill=(220,232,252,255))
d.rectangle([s(L),s(T+100),s(R),s(T+160)],fill=(220,232,252,255))
d.rounded_rectangle([s(440),s(228),s(768),s(278)],radius=s(25),fill=(26,82,190,255))
x0=256; maxw=768-256
rows=[(400,1.0),(462,0.94),(524,1.0),(586,0.88),(648,1.0),(710,0.96),(772,0.62)]
for i,(y,f) in enumerate(rows):
    w=maxw*f
    if i==3:
        d.rounded_rectangle([s(x0-12),s(y-14),s(x0+w+12),s(y+38)],radius=s(14),fill=(255,200,40,255))
        d.rounded_rectangle([s(x0),s(y),s(x0+w),s(y+24)],radius=s(12),fill=(90,90,110,255))
    else:
        d.rounded_rectangle([s(x0),s(y),s(x0+w),s(y+24)],radius=s(12),fill=(168,188,222,255))
cx,cy,r=292,272,116
d.ellipse([s(cx-r),s(cy-r),s(cx+r),s(cy+r)],fill=(255,255,255,255))
r2=r-18
d.ellipse([s(cx-r2),s(cy-r2),s(cx+r2),s(cy+r2)],fill=(16,62,152,255))
f=ImageFont.truetype("/usr/share/fonts/truetype/crosextra/Carlito-Bold.ttf",s(152))
bb=d.textbbox((0,0),"R",font=f)
d.text((s(cx)-(bb[0]+bb[2])/2,s(cy)-(bb[1]+bb[3])/2),"R",font=f,fill=(255,255,255,255))
img=img.resize((1024,1024),Image.LANCZOS)
img.save("rword-1024.png")
img.resize((256,256),Image.LANCZOS).save("rword-256.png")
open("rword-128.rgba","wb").write(img.resize((128,128),Image.LANCZOS).tobytes())
img.save("rword.ico",sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)])
img.save("rword.icns")
