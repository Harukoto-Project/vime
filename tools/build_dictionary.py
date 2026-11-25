#!/usr/bin/env python3
"""Builds the IME data in src/ime from the Mozc open source data.

Usage:
    python tools/build_dictionary.py <dir with the Mozc files>

The directory needs dictionary00..09.txt and id.def from
https://github.com/google/mozc/tree/master/src/data/dictionary_oss
and, for symbols, kaomoji and emoji, these files from the same repository:
symbol/symbol.tsv, emoticon/emoticon.tsv and emoji/emoji_data.tsv.

Outputs:
  src/ime/dictionary.tsv  reading<TAB>cost<TAB>token token ...
      The cost is the lowest Mozc cost of the reading (lower is more common). It
      ranks predictions and tells common readings from rare ones. A token is a one-letter class followed by the surface
      form: `n` for nouns and other stand-alone words, `w` for verbs and
      adjectives in their dictionary form, `v` for inflected verb and adjective
      forms that are followed by endings. Tokens of one reading are ordered from
      the most to the least likely.
  src/ime/symbols.tsv     reading<TAB>symbol symbol ...
      Symbols, kaomoji and emoji that can be typed by a Japanese reading.
"""

import collections
import pathlib
import re
import sys

MAX_COST = 7500
MAX_PER_READING = 24
READING = re.compile(r"^[ぁ-ゖー]+$")
KATAKANA = str.maketrans({chr(c): chr(c + 0x60) for c in range(0x3041, 0x3097)})
# Emoji newer than this may be missing from the fonts of the keyboard.
MAX_EMOJI_VERSION = 13.0
MAX_SYMBOLS_PER_READING = 120


def build_dictionary(source: pathlib.Path, out_dir: pathlib.Path) -> None:
    ids = {}
    for line in (source / "id.def").read_text(encoding="utf-8").splitlines():
        number, _, pos = line.partition(" ")
        ids[number] = pos.split(",")

    best = {}
    for path in sorted(source.glob("dictionary0*.txt")):
        for line in path.read_text(encoding="utf-8").splitlines():
            fields = line.split("\t")
            if len(fields) < 5:
                continue
            reading, left, _, cost, surface = fields[:5]
            cost = int(cost)
            if (
                cost > MAX_COST
                or surface == reading
                or not READING.match(reading)
                or surface == reading.translate(KATAKANA)
                or any(c.isspace() for c in surface)
                or left not in ids
            ):
                continue
            pos = ids[left]
            major = pos[0]
            if major in ("記号", "助詞", "助動詞", "フィラー", "その他"):
                continue
            if major == "名詞" and pos[1] == "固有名詞":
                # Keep common place and organisation names, drop most personal names.
                if pos[2] == "人名" and cost > 5200:
                    continue
                if cost > 6200:
                    continue
            if major in ("動詞", "形容詞"):
                kind = "w" if pos[5] == "基本形" else "v"
            else:
                kind = "n"
            key = (reading, kind + surface)
            if key not in best or cost < best[key]:
                best[key] = cost

    grouped = collections.defaultdict(list)
    for (reading, token), cost in best.items():
        grouped[reading].append((cost, token))

    # Hand-picked words first. They are common in VRChat conversations.
    extra = {}
    for line in (pathlib.Path(__file__).parent / "extra_words.tsv").read_text(
        encoding="utf-8"
    ).splitlines():
        reading, _, words = line.partition("\t")
        if reading:
            extra[reading] = ["n" + word for word in words.split(" ")]

    lines = []
    for reading in sorted(set(grouped) | set(extra)):
        tokens = list(extra.get(reading, []))
        ranked = sorted(grouped.get(reading, []))
        # Verb stems are only fragments of words, so they follow whole words.
        for _, token in sorted(ranked, key=lambda item: item[1][0] == "v"):
            if token not in tokens:
                tokens.append(token)
        cost = 0 if reading in extra else ranked[0][0]
        lines.append(f"{reading}\t{cost}\t{' '.join(tokens[:MAX_PER_READING])}")

    out = out_dir / "dictionary.tsv"
    out.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"{len(lines)} readings, {out.stat().st_size / 1e6:.2f} MB -> {out}")


def build_symbols(source: pathlib.Path, out_dir: pathlib.Path) -> None:
    by_reading = collections.defaultdict(list)

    def add(reading: str, symbol: str) -> None:
        if READING.match(reading) and symbol not in by_reading[reading]:
            by_reading[reading].append(symbol)

    symbol_file = source / "symbol.tsv"
    if symbol_file.exists():
        for line in symbol_file.read_text(encoding="utf-8").splitlines():
            if line.startswith("#"):
                continue
            fields = line.split("\t")
            if len(fields) < 3 or fields[0] in ("POS", ""):
                continue
            symbol = fields[1]
            # Letters are handled by the width conversions.
            if fields[0] == "アルファベット" or not symbol.strip():
                continue
            # Hieroglyphs, historic kana and the like are not worth the missing glyphs.
            if any(c.isspace() for c in symbol) or any(ord(c) > 0xFFFF for c in symbol):
                continue
            for reading in fields[2].split(" "):
                add(reading, symbol)

    emoticon_file = source / "emoticon.tsv"
    if emoticon_file.exists():
        for line in emoticon_file.read_text(encoding="utf-8").splitlines()[1:]:
            fields = line.split("\t")
            if len(fields) < 2 or not fields[0] or any(c.isspace() for c in fields[0]):
                continue
            for reading in fields[1].split(" "):
                add(reading, fields[0])

    emoji_file = source / "emoji_data.tsv"
    if emoji_file.exists():
        for line in emoji_file.read_text(encoding="utf-8").splitlines():
            if line.startswith("#"):
                continue
            fields = line.split("\t")
            if len(fields) < 7:
                continue
            points = [int(point, 16) for point in fields[0].split(" ")]
            version = re.match(r"E(\d+\.\d+)", fields[6])
            if (
                not version
                or float(version.group(1)) > MAX_EMOJI_VERSION
                # Flags, skin tones and joined sequences need font support that
                # the keyboard does not have.
                or 0x200D in points
                or any(0x1F1E6 <= p <= 0x1F1FF or 0x1F3FB <= p <= 0x1F3FF for p in points)
                or points[0] < 0x80
            ):
                continue
            for reading in fields[2].split(" "):
                # The variation selector only draws as a box in the keyboard.
                add(reading, fields[1].replace("\uFE0F", ""))

    lines = [
        f"{reading}\t{' '.join(symbols[:MAX_SYMBOLS_PER_READING])}"
        for reading, symbols in sorted(by_reading.items())
    ]
    out = out_dir / "symbols.tsv"
    out.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"{len(lines)} readings, {out.stat().st_size / 1e6:.2f} MB -> {out}")


def main(source: pathlib.Path) -> None:
    out_dir = pathlib.Path(__file__).parent.parent / "src" / "ime"
    build_dictionary(source, out_dir)
    build_symbols(source, out_dir)


if __name__ == "__main__":
    main(pathlib.Path(sys.argv[1]))
