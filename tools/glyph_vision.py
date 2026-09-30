#!/usr/bin/env python3
"""Identify unmapped VT52x glyphs with a vision model.

Reads the glyph review page written by
`GLYPHS_REVIEW=review.html cargo test --release --lib -- --ignored glyph_review`,
renders each glyph the table does not know as an enlarged PNG, asks a model on
an OpenAI-compatible endpoint for the character, and writes
`src/machine/vt52x/glyphs-vision.txt`. Answers are cached in a JSON file so a
run can resume.

Usage:
  glyph_vision.py review.html [--calibrate N] [--limit N]

--calibrate N asks about N glyphs the terminal already labelled and reports
how often the model agrees, instead of writing the table.
"""

import argparse
import base64
import concurrent.futures
import json
import os
import random
import struct
import unicodedata
import urllib.request
import zlib

ENDPOINT = os.environ.get("VISION_URL", "http://192.168.3.7:6381/v1/chat/completions")
MODEL = os.environ.get("VISION_MODEL", "qwen36-a3b-128k")
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TABLE = os.path.join(ROOT, "src/machine/vt52x/glyphs-vision.txt")
CACHE = os.path.join(ROOT, "tools/glyph_vision_cache.json")

PROMPT = (
    "This image shows one glyph from a DEC VT terminal font, black on white, "
    "enlarged. It may be a Latin, Greek, Cyrillic or Hebrew letter, a digit, "
    "punctuation, a line-drawing or block piece, a PC code page 437 symbol, "
    "a math or technical symbol, or a piece of a larger bracket or integral. "
    "Reply with only the single Unicode character it shows, nothing else. "
    "If it is not a character, reply with ?"
)


def fingerprint(lines):
    """The same FNV-1a hash as src/machine/generic/glyphs.rs."""
    h = 0xCBF29CE484222325

    def add(byte):
        nonlocal h
        h ^= byte
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF

    add(len(lines) & 0xFF)
    for line in lines:
        add(line & 0xFF)
        add(line >> 8)
    return h


def glyph_png(lines, width=10, scale=8, margin=2):
    height = len(lines)
    w, h = (width + 2 * margin) * scale, (height + 2 * margin) * scale
    rows = [[255] * w for _ in range(h)]
    for y, bits in enumerate(lines):
        for x in range(width):
            if bits >> x & 1:
                for dy in range(scale):
                    row = rows[(y + margin) * scale + dy]
                    for dx in range(scale):
                        row[(x + margin) * scale + dx] = 0
    raw = b"".join(b"\x00" + bytes(r) for r in rows)

    def chunk(kind, data):
        return (
            struct.pack(">I", len(data))
            + kind
            + data
            + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
        )

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 0, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )


def ask(lines):
    image = base64.b64encode(glyph_png(lines)).decode()
    body = {
        "model": MODEL,
        "max_tokens": 16,
        "temperature": 0,
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64," + image}},
                    {"type": "text", "text": PROMPT},
                ],
            }
        ],
    }
    request = urllib.request.Request(
        ENDPOINT, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"}
    )
    reply = json.load(urllib.request.urlopen(request, timeout=300))
    text = (reply["choices"][0]["message"]["content"] or "").strip()
    # One character, allowing a combining mark after it.
    if not text or text == "?":
        return None
    chars = [c for c in text if not unicodedata.combining(c)]
    if len(chars) != 1 or len(text) > 2:
        return None
    return unicodedata.normalize("NFC", text)


def load_glyphs(review):
    page = open(review, encoding="utf-8").read()
    data = json.loads(page[page.index("const DATA = ") + 13 : page.index(";\nconst out")])
    glyphs = {}
    for _, fonts in data:
        for _, entries in fonts:
            for code, label, lines, source, font in entries:
                if label is None:
                    continue
                glyphs.setdefault(fingerprint(lines), (lines, label, source))
    return glyphs


def run(glyphs, cache, jobs):
    todo = [h for h in glyphs if str(h) not in cache]
    with concurrent.futures.ThreadPoolExecutor(jobs) as pool:
        futures = {pool.submit(ask, glyphs[h][0]): h for h in todo}
        for i, future in enumerate(concurrent.futures.as_completed(futures), 1):
            h = futures[future]
            try:
                cache[str(h)] = future.result()
            except Exception as error:  # Leave it for the next run.
                print("error", hex(h), error)
            if i % 100 == 0:
                print(f"{i}/{len(todo)}")
                json.dump(cache, open(CACHE, "w"), ensure_ascii=False)
    json.dump(cache, open(CACHE, "w"), ensure_ascii=False)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("review")
    parser.add_argument("--calibrate", type=int, default=0)
    parser.add_argument("--limit", type=int, default=0)
    parser.add_argument("--jobs", type=int, default=8)
    args = parser.parse_args()

    glyphs = load_glyphs(args.review)
    cache = json.load(open(CACHE)) if os.path.exists(CACHE) else {}

    if args.calibrate:
        known = [h for h, (_, label, source) in glyphs.items() if label and source == "e"]
        random.seed(1)
        sample = {h: glyphs[h] for h in random.sample(known, min(args.calibrate, len(known)))}
        run(sample, cache, args.jobs)
        agree = [h for h in sample if cache.get(str(h)) == sample[h][1]]
        answered = [h for h in sample if cache.get(str(h))]
        print(f"calibration: {len(agree)}/{len(sample)} agree, {len(answered)} answered")
        for h in sample:
            if cache.get(str(h)) != sample[h][1]:
                print(f"  {h:016x} terminal {sample[h][1]!r} model {cache.get(str(h))!r}")
        return

    unmapped = {h: g for h, g in glyphs.items() if g[1] == "" or g[2] == "v"}
    if args.limit:
        unmapped = dict(list(unmapped.items())[: args.limit])
    run(unmapped, cache, args.jobs)
    lines = [
        "# Glyphs no character set reaches, identified from images by a vision model",
        f"# ({MODEL}) with tools/glyph_vision.py. Check them in the review page.",
    ]
    for h in sorted(unmapped):
        ch = cache.get(str(h))
        if ch:
            lines.append(f"{h:016x} U+{ord(ch[0]):04X}")
    open(TABLE, "w").write("\n".join(lines) + "\n")
    print(f"{len(lines) - 2} of {len(unmapped)} glyphs identified")


if __name__ == "__main__":
    main()
