"""Write the app's icon files from crates/app/assets/icon.png.

Needs Pillow (`pip install pillow`). Run, from the repository root:

    python tools/make_icon.py

It writes two files next to the PNG, both built into the exe:

- icon.res: a compiled Windows resource holding the icon at several sizes.
  The build script hands it to the linker, so the exe file itself carries
  the icon (Explorer, a pinned taskbar button and shortcuts read it from
  there). It is written here rather than compiled from an .rc file so the
  build needs no resource compiler or extra library.
- icon.rgba: the PNG's pixels, unpacked, for the window's own icon while
  the app runs.

Re-run it whenever icon.png changes.
"""
import os
import struct
import sys

from PIL import Image

ASSETS = os.path.join("crates", "app", "assets")
SOURCE = os.path.join(ASSETS, "icon.png")
SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]

RT_ICON = 3
RT_GROUP_ICON = 14
LANGUAGE = 0x0409  # US English, what the resource compiler uses by default
ICON_FLAGS = 0x1010  # moveable, discardable
GROUP_FLAGS = 0x1030  # moveable, pure, discardable


def dib(image):
    """One icon image as Windows stores it: a 32-bit bitmap, bottom row
    first, followed by a one-bit mask that marks the see-through pixels."""
    w, h = image.size
    pixels = image.load()
    colour = bytearray()
    for y in reversed(range(h)):
        for x in range(w):
            r, g, b, a = pixels[x, y]
            colour += bytes((b, g, r, a))
    mask_row = ((w + 31) // 32) * 4
    mask = bytearray()
    for y in reversed(range(h)):
        row = bytearray(mask_row)
        for x in range(w):
            if pixels[x, y][3] == 0:
                row[x // 8] |= 0x80 >> (x % 8)
        mask += row
    # The height counts the colour and the mask together.
    header = struct.pack(
        "<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, len(colour) + len(mask), 0, 0, 0, 0
    )
    return header + bytes(colour) + bytes(mask)


def resource(kind, ident, flags, data):
    """One entry of a .res file, padded to four bytes."""
    header = struct.pack(
        "<IIHHHHIHHII",
        len(data),
        32,
        0xFFFF,
        kind,
        0xFFFF,
        ident,
        0,
        flags,
        LANGUAGE,
        0,
        0,
    )
    padding = b"\0" * (-len(data) % 4)
    return header + data + padding


def main():
    source = Image.open(SOURCE).convert("RGBA")
    width, height = source.size
    if width != height:
        sys.exit(f"{SOURCE} must be square, not {width}x{height}")
    # Larger sizes than the source would only be blurred copies of it.
    sizes = [size for size in SIZES if size <= width]
    if width not in sizes:
        sizes.append(width)

    # The empty entry every .res file starts with.
    res = resource(0, 0, 0, b"")
    group = struct.pack("<HHH", 0, 1, len(sizes))
    for ident, size in enumerate(sizes, start=1):
        image = source if size == width else source.resize((size, size), Image.LANCZOS)
        data = dib(image)
        res += resource(RT_ICON, ident, ICON_FLAGS, data)
        # A width or height of 256 is written as 0.
        side = size % 256
        group += struct.pack("<BBBBHHIH", side, side, 0, 0, 1, 32, len(data), ident)
    res += resource(RT_GROUP_ICON, 1, GROUP_FLAGS, group)

    with open(os.path.join(ASSETS, "icon.res"), "wb") as f:
        f.write(res)
    with open(os.path.join(ASSETS, "icon.rgba"), "wb") as f:
        f.write(source.tobytes())
    print(f"icon.res: sizes {sizes}; icon.rgba: {width}x{height}")


if __name__ == "__main__":
    main()
