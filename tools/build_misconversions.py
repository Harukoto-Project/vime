#!/usr/bin/env python3
"""Reference for the misconversion table that ViME builds on the user's PC.

ViME downloads the Japanese Wikipedia Typo Dataset (JWTD v2, Kyoto University
Language Media Processing Lab, CC BY-SA 3.0) itself and counts it with
src/ime/misconversion.rs. The table is never part of this repository. This
script makes the same count in Python and measures thresholds; keep the two
in step.

Usage:
    python tools/build_misconversions.py <jwtd_v2.0 dir> [--evaluate] [--output FILE]

The dataset comes from
https://nlp.ist.i.kyoto-u.ac.jp/nl-resource/JWTD/jwtd_v2.0.tar.gz
Only the kanji misconversions (`kanji-conversion_a` and `_b`) of train.jsonl
are used, and no sentences are kept. Output lines are
`word<TAB>corrected<TAB>kept<TAB>fix fix fix`: how often editors replaced the
word, how often it stays in the corrected sentences, and the most frequent
replacements. `--evaluate` measures the table on test.jsonl and on the manually
checked gold.jsonl (the set used in Asahi Shimbun's LLM typo benchmark); neither is
used to build it. The output derives from a CC BY-SA 3.0 dataset, so do not commit
it or share it under another license.
"""

import collections
import difflib
import json
import pathlib
import re
import sys

CATEGORIES = {"kanji-conversion_a", "kanji-conversion_b"}
MIN_LENGTH = 2
MAX_LENGTH = 6
MAX_FIXES = 3
# The share of corrected uses below which Rust does not store a word
# (STORE_RATIO in src/ime/misconversion.rs).
MIN_CORRECTED = 1
MIN_RATIO = 0.1
KANJI = re.compile(r"[\u3400-\u4dbf\u4e00-\u9fff々〆]")
WORD = re.compile(r"^[^\s\t]+$")


def records(path: pathlib.Path):
    with path.open(encoding="utf-8") as lines:
        for line in lines:
            yield json.loads(line)


def usable(word: str) -> bool:
    return (
        MIN_LENGTH <= len(word) <= MAX_LENGTH
        and KANJI.search(word) is not None
        and WORD.match(word) is not None
    )


def count_corrections(path: pathlib.Path):
    corrected = collections.Counter()
    fixes = collections.defaultdict(collections.Counter)
    for record in records(path):
        for diff in record.get("diffs", []):
            if diff.get("category") not in CATEGORIES:
                continue
            wrong, fix = diff["pre_str"], diff["post_str"]
            if wrong != fix and usable(wrong) and usable(fix):
                corrected[wrong] += 1
                fixes[wrong][fix] += 1
    return corrected, fixes


def count_kept(path: pathlib.Path, words: set[str]):
    lengths = sorted({len(word) for word in words})
    kept = collections.Counter()
    for record in records(path):
        text = record["post_text"]
        for length in lengths:
            for start in range(len(text) - length + 1):
                piece = text[start : start + length]
                if piece in words:
                    kept[piece] += 1
    return kept


def build(source: pathlib.Path):
    train = source / "train.jsonl"
    corrected, fixes = count_corrections(train)
    words = {word for word, count in corrected.items() if count >= MIN_CORRECTED}
    kept = count_kept(train, words)
    table = {}
    for word in words:
        ratio = corrected[word] / (corrected[word] + kept[word])
        if ratio >= MIN_RATIO:
            best = [fix for fix, _ in fixes[word].most_common(MAX_FIXES)]
            table[word] = (corrected[word], kept[word], best)
    return table


def write(table, output: pathlib.Path) -> None:
    lines = [
        f"{word}\t{wrong}\t{kept}\t{' '.join(fixes)}\n"
        for word, (wrong, kept, fixes) in sorted(table.items())
    ]
    output.write_text("".join(lines), encoding="utf-8", newline="\n")


def evaluate(table, source: pathlib.Path) -> None:
    found = covered = total = 0
    for record in records(source / "test.jsonl"):
        for diff in record.get("diffs", []):
            if diff.get("category") not in CATEGORIES:
                continue
            total += 1
            entry = table.get(diff["pre_str"])
            if entry:
                found += 1
                covered += diff["post_str"] in entry[2]
    print(f"test misconversions: {total}")
    print(f"  word in table:        {found} ({found / total:.1%})")
    print(f"  right fix suggested:  {covered} ({covered / total:.1%})")
    for threshold in (0.05, 0.1, 0.2, 0.3, 0.5):
        gated = {
            word
            for word, (wrong, kept, _) in table.items()
            if wrong / (wrong + kept) >= threshold
        }
        lengths = sorted({len(word) for word in gated})
        sentences = fired = 0
        for record in records(source / "test.jsonl"):
            text = record["post_text"]
            sentences += 1
            hit = False
            for length in lengths:
                for start in range(len(text) - length + 1):
                    if text[start : start + length] in gated:
                        hit = True
                        break
                if hit:
                    break
            fired += hit
        right = sum(
            1
            for record in records(source / "test.jsonl")
            for diff in record.get("diffs", [])
            if diff.get("category") in CATEGORIES and diff["pre_str"] in gated
        )
        print(
            f"  ratio >= {threshold:.2f}: {len(gated):6d} words, "
            f"{right / total:.1%} of misconversions reachable, "
            f"{fired / sentences:.1%} of correct sentences would be asked about"
        )


def kanji_replacements(pre: str, post: str):
    """Spans where `pre` has other kanji text than `post` and nothing else differs."""
    matcher = difflib.SequenceMatcher(None, pre, post, autojunk=False)
    changes = [op for op in matcher.get_opcodes() if op[0] != "equal"]
    if len(changes) != 1 or changes[0][0] != "replace":
        return None
    _, start, end, post_start, post_end = changes[0]
    if KANJI.search(pre[start:end]) and KANJI.search(post[post_start:post_end]):
        return start, end
    return None


def evaluate_gold(table, source: pathlib.Path) -> None:
    """The manually checked `gold` set (as in Asahi Shimbun's benchmark).

    It has no error labels, so a sentence counts as a kanji misconversion when
    exactly one kanji span was replaced. A fix is offered when a table word that
    overlaps the span lists a replacement that gives the corrected sentence.
    """
    gold = list(records(source / "gold.jsonl"))
    cases = [
        (record, span)
        for record in gold
        if (span := kanji_replacements(record["pre_text"], record["post_text"]))
    ]
    print(f"gold sentences: {len(gold)}; single kanji replacements: {len(cases)}")
    for threshold in (0.1, 0.2, 0.3, 0.5):
        words = {
            word: fixes
            for word, (wrong, kept, fixes) in table.items()
            if wrong / (wrong + kept) >= threshold
        }
        lengths = sorted({len(word) for word in words})

        def hits(text):
            for length in lengths:
                for start in range(len(text) - length + 1):
                    word = text[start : start + length]
                    if word in words:
                        yield start, start + length, words[word]

        reached = fixed = 0
        for record, (start, end) in cases:
            pre, post = record["pre_text"], record["post_text"]
            found = [h for h in hits(pre) if h[0] < end and h[1] > start]
            reached += bool(found)
            fixed += any(
                pre[: s] + fix + pre[e:] == post for s, e, fixes in found for fix in fixes
            )
        asked = sum(1 for record in gold if next(hits(record["post_text"]), None))
        print(
            f"  ratio >= {threshold:.2f}: word found {reached / len(cases):.1%}, "
            f"right sentence offered {fixed / len(cases):.1%}, "
            f"corrected sentences asked about {asked / len(gold):.1%}"
        )


def main(source: pathlib.Path, check: bool, output: pathlib.Path | None) -> None:
    table = build(source)
    print(f"{len(table)} words")
    if output:
        write(table, output)
        print(f"written to {output} ({output.stat().st_size / 1e6:.2f} MB)")
    if check:
        evaluate(table, source)
        evaluate_gold(table, source)


if __name__ == "__main__":
    arguments = sys.argv[1:]
    output = None
    if "--output" in arguments:
        index = arguments.index("--output")
        if index + 1 >= len(arguments):
            sys.exit(__doc__)
        output = pathlib.Path(arguments[index + 1])
        del arguments[index : index + 2]
    args = [argument for argument in arguments if not argument.startswith("--")]
    if len(args) != 1:
        sys.exit(__doc__)
    main(pathlib.Path(args[0]), "--evaluate" in arguments, output)
