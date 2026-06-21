#!/usr/bin/env python3
"""Generate NetCheck-Windows icons (app .ico + PNG set + coloured tray globes).

Draws the "living globe" mark (concept A from the Mac app): a deep gradient
squircle with a glossy meridian globe. The tray variants reuse the same globe
in the four status colours so the tray icon itself carries the status — on
Windows the tray text/title is a no-op, so colour is the only channel.

Pure Pillow, no external rasteriser. Run once and commit src-tauri/icons/.
"""
import math
import os
from PIL import Image, ImageDraw, ImageFilter

ICONS = os.path.join(os.path.dirname(__file__), "..", "src-tauri", "icons")
os.makedirs(ICONS, exist_ok=True)

# status colour themes: (top RGB, bottom RGB) for the squircle gradient
THEMES = {
    "green": ((43, 179, 107), (7, 92, 51)),     # alive / fast / normal — the brand default
    "amber": ((245, 180, 80), (140, 80, 8)),    # slow / sign-in needed
    "red":   ((235, 90, 90), (120, 25, 25)),    # offline
    "slate": ((140, 160, 180), (50, 62, 78)),   # checking
}

# Tray icons are keyed by STATE (not just colour) so the shape carries meaning too —
# a slash for offline, a badge for sign-in — for colour-blind glanceability.
TRAY_STATES = {
    "online":   ("green", None),
    "slow":     ("amber", None),
    "portal":   ("amber", "badge"),
    "offline":  ("red", "slash"),
    "checking": ("slate", None),
}


def _lerp(a, b, t):
    return int(round(a + (b - a) * t))


def _gradient(size, top, bottom):
    col = Image.new("RGBA", (1, size))
    for y in range(size):
        t = y / (size - 1)
        col.putpixel((0, y), (_lerp(top[0], bottom[0], t),
                              _lerp(top[1], bottom[1], t),
                              _lerp(top[2], bottom[2], t), 255))
    return col.resize((size, size))


def _squircle_mask(size):
    m = Image.new("L", (size, size), 0)
    ImageDraw.Draw(m).rounded_rectangle(
        [0, 0, size - 1, size - 1], radius=int(size * 0.225), fill=255)
    return m


def _globe(size, tray=False):
    """White meridian globe on a transparent layer."""
    layer = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    cx = cy = size / 2.0
    R = size * (0.37 if tray else 0.33)
    w = max(1, int(round(size * (0.045 if tray else 0.022))))
    a = 235 if tray else 218
    line = (255, 255, 255, a)

    d.ellipse([cx - R, cy - R, cx + R, cy + R], outline=line, width=w)        # rim
    rx = R * 0.55
    d.ellipse([cx - rx, cy - R, cx + rx, cy + R], outline=line, width=w)      # meridian
    d.line([cx, cy - R, cx, cy + R], fill=line, width=w)                      # centre meridian
    d.line([cx - R, cy, cx + R, cy], fill=line, width=w)                      # equator
    if not tray:                                                             # parallels
        off = R * 0.5
        hw = R * math.cos(math.asin(0.5))
        for dy in (-off, off):
            d.line([cx - hw, cy + dy, cx + hw, cy + dy], fill=line, width=w)
    return layer


def _draw_mark(img, size, mark):
    """Shape cue so status isn't conveyed by colour alone: slash = offline, badge = sign-in."""
    if not mark:
        return
    d = ImageDraw.Draw(img)
    c = size / 2.0
    R = size * 0.37
    if mark == "slash":
        w = max(2, int(round(size * 0.08)))
        d.line([c - R, c - R, c + R, c + R], fill=(15, 15, 15, 190), width=w + max(2, int(size * 0.04)))
        d.line([c - R, c - R, c + R, c + R], fill=(255, 255, 255, 245), width=w)
    elif mark == "badge":
        r = size * 0.17
        bx, by = size * 0.73, size * 0.27
        d.ellipse([bx - r - 1, by - r - 1, bx + r + 1, by + r + 1], fill=(15, 15, 15, 170))
        d.ellipse([bx - r, by - r, bx + r, by + r], fill=(255, 255, 255, 245))


def render(size, theme, tray=False, mark=None):
    top, bottom = THEMES[theme]
    base = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    base.paste(_gradient(size, top, bottom), (0, 0), _squircle_mask(size))

    if not tray:  # soft top sheen for the glass look
        gloss = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        ImageDraw.Draw(gloss).ellipse(
            [size * 0.16, size * 0.08, size * 0.84, size * 0.56], fill=(255, 255, 255, 60))
        gloss = gloss.filter(ImageFilter.GaussianBlur(size * 0.06))
        gloss.putalpha(gloss.getchannel("A").point(lambda p: int(p * 0.7)))
        base = Image.alpha_composite(base, Image.composite(
            gloss, Image.new("RGBA", (size, size), (0, 0, 0, 0)), _squircle_mask(size)))

    img = Image.alpha_composite(base, _globe(size, tray=tray))
    _draw_mark(img, size, mark)
    return img


def main():
    master = render(1024, "green")
    # app icon PNG set
    for px, name in [(1024, "icon.png"), (512, "512x512.png"), (256, "256x256.png"),
                     (256, "128x128@2x.png"), (128, "128x128.png"), (32, "32x32.png")]:
        master.resize((px, px), Image.LANCZOS).save(os.path.join(ICONS, name))
    # multi-resolution Windows .ico
    render(256, "green").save(
        os.path.join(ICONS, "icon.ico"),
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    # state-keyed tray globes (colour + shape: slash = offline, badge = sign-in)
    for state, (theme, mark) in TRAY_STATES.items():
        render(64, theme, tray=True, mark=mark).save(os.path.join(ICONS, f"tray-{state}.png"))
    print("wrote icons to", os.path.normpath(ICONS))
    for f in sorted(os.listdir(ICONS)):
        print("  ", f)


if __name__ == "__main__":
    main()
