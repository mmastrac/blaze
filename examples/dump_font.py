#!/usr/bin/env python3
import argparse, sys, os

def render_to_bitmap(data, bits_top_is_msb=False):
    """
    Interpret `data` as N chunks of 256 bytes.
    Each byte => vertical column of 8 pixels.
    Returns a 2D list of 0/255 with shape (height, 256).
    """
    if len(data) % 256 != 0:
        # Trim trailing partial chunk (common with raw dumps)
        data = data[: len(data) // 256 * 256]

    n_chunks = len(data) // 256
    width, height = 256, 8 * n_chunks
    img = [[0]*width for _ in range(height)]

    for chunk_idx in range(n_chunks):
        base = chunk_idx * 256
        for x in range(256):
            b = data[base + x]
            # Row 0 is the top of this 8-line band
            row_base = chunk_idx * 8
            for bit in range(8):
                if bits_top_is_msb:
                    val = (b >> (7 - bit)) & 1
                else:
                    val = (b >> bit) & 1
                img[row_base + bit][x] = 255 if val else 0
    return img, width, height

def save_png(path, img, w, h):
    try:
        from PIL import Image
    except ImportError:
        return False
    im = Image.new("L", (w, h))
    # Flatten rows
    im.putdata([pix for row in img for pix in row])
    im.save(path)
    return True

def save_pbm_ascii(path, img, w, h):
    # Portable bitmap (P1): 1=black, 0=white; we’ll invert so set bits are black.
    with open(path, "w", newline="\n") as f:
        f.write(f"P1\n{w} {h}\n")
        for row in img:
            # 255->1 (black), 0->0 (white)
            f.write(" ".join("1" if p else "0" for p in row) + "\n")

def main():
    ap = argparse.ArgumentParser(
        description="Render VT420-style 256-byte chunks (8 rows of vertical bit columns) to an image."
    )
    ap.add_argument("infile", help="binary VRAM dump")
    ap.add_argument("outfile", help="output image (PNG if Pillow installed, else PBM)")
    ap.add_argument("--offset", type=lambda s:int(s,0), default=0x8000,
                    help="start offset into file (e.g. 0x8000) [default: 0x8000]")
    ap.add_argument("--chunks", type=int, default=None,
                    help="limit number of 256-byte chunks to decode")
    ap.add_argument("--msb-top", action="store_true",
                    help="use MSB at top of the 8-pixel column (default: LSB at top)")
    args = ap.parse_args()

    with open(args.infile, "rb") as f:
        f.seek(args.offset, os.SEEK_SET)
        data = f.read()

    if args.chunks is not None:
        data = data[: args.chunks * 256]

    img, w, h = render_to_bitmap(data, bits_top_is_msb=args.msb_top)

    # Try PNG, fall back to PBM if Pillow missing
    if not save_png(args.outfile, img, w, h):
        # If requested .png but Pillow’s missing, switch extension to .pbm
        out = args.outfile
        if out.lower().endswith(".png"):
            out = out[:-4] + ".pbm"
            print(f"Pillow not found; writing PBM instead: {out}")
        save_pbm_ascii(out, img, w, h)

if __name__ == "__main__":
    main()