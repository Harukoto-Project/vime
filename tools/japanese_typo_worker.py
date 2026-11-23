"""Resident LINE DistilBERT worker. stdin/stdout carry UTF-8 JSON lines."""

import json
from pathlib import Path
import sys
import unicodedata

import fugashi
import numpy as np
import onnxruntime as ort
import regex
import sentencepiece as spm
import unidic_lite


JAPANESE = regex.compile(r"[\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Han}ー]")
REPLACEMENT = regex.compile(r"[\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Han}ー々]+\Z")
MASK_SPACING = 5
VERIFY_BATCH = 3
SEQUENCE_BUCKETS = (16, 32, 64, 128)


class Detector:
    def __init__(self, directory, adapter):
        dictionary = Path(unidic_lite.DICDIR)
        self.mecab = fugashi.GenericTagger(f'-d "{dictionary}" -r "{dictionary / "mecabrc"}"')
        self.spm = spm.SentencePieceProcessor(model_file=str(directory / "spiece.model"))
        self.cls = self.spm.piece_to_id("[CLS]")
        self.sep = self.spm.piece_to_id("[SEP]")
        self.mask = self.spm.piece_to_id("[MASK]")
        self.pad = self.spm.pad_id()
        if any(token == self.spm.unk_id() for token in (self.cls, self.sep, self.mask)):
            raise ValueError("LINE tokenizer special tokens are missing")
        self.replacements = {}
        for token in range(self.spm.get_piece_size()):
            piece = self.spm.id_to_piece(token)
            surface = piece.removeprefix("▁")
            if REPLACEMENT.fullmatch(surface):
                self.replacements[token] = (piece.startswith("▁"), surface)
        self.vocabulary = {
            boundary: np.array([
                token for token, (starts_word, _) in self.replacements.items()
                if starts_word == boundary
            ], dtype=np.int64)
            for boundary in (False, True)
        }
        options = ort.SessionOptions()
        options.enable_mem_pattern = False
        options.intra_op_num_threads = 2
        options.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
        options.log_severity_level = 3
        self.session = ort.InferenceSession(
            str(directory / "masked-lm-fp16.onnx"), sess_options=options,
            providers=[("DmlExecutionProvider", {"device_id": adapter})],
        )
        if self.session.get_providers()[0] != "DmlExecutionProvider":
            raise RuntimeError("ONNX Runtime could not initialize the DirectML adapter")
        self.session.disable_fallback()
        for length in SEQUENCE_BUCKETS:
            ids = [self.cls] + [self.mask] * (length - 2) + [self.sep]
            self.score(ids, [list(range(row, length, MASK_SPACING))
                             for row in range(MASK_SPACING)], MASK_SPACING, length)
            self.score(ids, [[1], [2], [3]], VERIFY_BATCH, VERIFY_BATCH)

    def tokenize(self, text):
        # Map only complete grapheme boundaries back to UTF-8 editor offsets.
        # A token that would split a normalized grapheme is context-only.
        normalized = []
        offsets = {0: 0}
        chars = byte = 0
        for cluster in regex.findall(r"\X", text):
            part = unicodedata.normalize("NFKC", cluster)
            normalized.append(part)
            chars += len(part)
            byte += len(cluster.encode("utf-8"))
            offsets[chars] = byte
        normalized = "".join(normalized)
        ids = [self.cls]
        positions = {}
        cursor = 0
        for word in self.mecab(normalized):
            start = normalized.index(word.surface, cursor)
            cursor = start + len(word.surface)
            surface = word.surface.lower().replace("``", '"').replace("''", '"')
            for piece in self.spm.encode(surface, out_type="immutable_proto").pieces:
                index = len(ids)
                ids.append(piece.id)
                if (len(surface) == len(word.surface) and piece.begin < piece.end
                        and surface[piece.begin:piece.end] == piece.surface
                        and JAPANESE.search(piece.surface)
                        and (left := offsets.get(start + piece.begin)) is not None
                        and (right := offsets.get(start + piece.end)) is not None):
                    positions[index] = (left, right)
        ids.append(self.sep)
        return ids, positions

    def score(self, ids, groups, batch_size, output_size=None):
        length = next(size for size in SEQUENCE_BUCKETS if size >= len(ids))
        masked = np.full((batch_size, length), self.pad, dtype=np.int64)
        masked[:, :len(ids)] = ids
        attention = np.zeros_like(masked)
        attention[:, :len(ids)] = 1
        indexes = np.zeros(output_size or length, dtype=np.int64)
        offsets = []
        for row, positions in enumerate(groups):
            masked[row, positions] = self.mask
            offsets.extend(row * length + position for position in positions)
        indexes[:len(offsets)] = offsets
        logits, = self.session.run(None, {
            "input_ids": masked,
            "attention_mask": attention,
            "positions": indexes,
        })
        return logits[:len(offsets)].astype(np.float32, copy=False)

    @staticmethod
    def probabilities(rows, originals):
        ratios = np.exp(rows - rows.max(axis=1, keepdims=True))
        return ratios[np.arange(len(rows)), originals] / ratios.sum(axis=1)

    def detect(self, text, caret, threshold, count, max_positions):
        ids, spans = self.tokenize(text)
        if not spans or len(ids) > SEQUENCE_BUCKETS[-1]:
            return []
        # Spaced masks let all positions share a few forward passes. Verify
        # shortlisted positions with one mask per sequence for the final scores.
        groups = [[index for index in spans if index % MASK_SPACING == row]
                  for row in range(MASK_SPACING)]
        indexes = [index for group in groups for index in group]
        rows = self.score(ids, groups, MASK_SPACING)
        probabilities = self.probabilities(rows, [ids[index] for index in indexes])
        suspects = sorted(
            (float(p), abs(spans[index][1] - caret), index)
            for index, p in zip(indexes, probabilities, strict=True) if p < threshold
        )
        indexes = [index for _, _, index in suspects[:max_positions]]
        if not indexes:
            return []
        rows = self.score(ids, [[index] for index in indexes], VERIFY_BATCH, VERIFY_BATCH)
        probabilities = self.probabilities(rows, [ids[index] for index in indexes])
        detections = []
        source = text.encode("utf-8")
        for index, row, probability in zip(indexes, rows, probabilities, strict=True):
            if probability >= threshold:
                continue
            original = ids[index]
            left, right = spans[index]
            surface = source[left:right].decode("utf-8")
            starts_word = self.spm.id_to_piece(original).startswith("▁")
            vocabulary = self.vocabulary[starts_word]
            scores = row[vocabulary]
            size = min(count + 1, len(vocabulary))
            top = np.argpartition(scores, -size)[-size:]
            choices = [
                self.replacements[vocabulary[i]][1]
                for i in top[np.argsort(scores[top])[::-1]]
                if vocabulary[i] != original and scores[i] > row[original]
                and self.replacements[vocabulary[i]][1] != surface
            ][:count]
            if choices:
                detections.append({"start": left, "end": right, "alternatives": choices})
        return detections


def send(value):
    print(json.dumps(value, ensure_ascii=True), flush=True)


if __name__ == "__main__":
    try:
        detector = Detector(Path(sys.argv[1]), int(sys.argv[2]))
        send({"ready": True})
    except Exception as error:
        send({"error": str(error)})
        sys.exit(1)
    for line in sys.stdin:
        try:
            send({"hits": detector.detect(**json.loads(line))})
        except Exception as error:
            send({"error": str(error)})
