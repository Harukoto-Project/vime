#!/usr/bin/env python3
"""Builds src/ime/dictionary.tsv from the Mozc open source dictionary.

Usage:
    python tools/build_dictionary.py <dir with dictionary00..09.txt and id.def>

The Mozc files come from
https://github.com/google/mozc/tree/master/src/data/dictionary_oss
Output lines are `reading<TAB>token token ...`. A token is a one-letter class
followed by the surface form: `n` for nouns and other stand-alone words, `v` for
inflected verb and adjective forms that are followed by endings. Tokens of one
reading are ordered from the most to the least likely.
"""

import collections
import pathlib
import re
import sys

MAX_COST = 6000
MAX_PER_READING = 6
READING = re.compile(r"^[ぁ-ゖー]+$")
KATAKANA = str.maketrans({chr(c): chr(c + 0x60) for c in range(0x3041, 0x3097)})


def main(source: pathlib.Path) -> None:
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
                if pos[2] == "人名" and cost > 4600:
                    continue
                if cost > 5500:
                    continue
            kind = "v" if major in ("動詞", "形容詞") else "n"
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
        for _, token in sorted(grouped.get(reading, [])):
            if token not in tokens:
                tokens.append(token)
        lines.append(f"{reading}\t{' '.join(tokens[:MAX_PER_READING])}")

    out = pathlib.Path(__file__).parent.parent / "src" / "ime" / "dictionary.tsv"
    out.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"{len(lines)} readings, {out.stat().st_size / 1e6:.2f} MB -> {out}")


if __name__ == "__main__":
    main(pathlib.Path(sys.argv[1]))
