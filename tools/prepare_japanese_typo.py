"""Export the LINE checkpoint into ViME's local DirectML model directory."""

import argparse
import os
from pathlib import Path
import shutil
import tempfile
from urllib.request import urlretrieve

import onnx
from onnxconverter_common import float16
import torch
from transformers import DistilBertForMaskedLM


MODEL = "line-corporation/line-distilbert-base-japanese"
REVISION = "c32d05753d0d4be0f60482291a9452ec7e6f7d68"


class MaskedRows(torch.nn.Module):
    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, input_ids, attention_mask, positions):
        model = self.model
        hidden = model.distilbert(input_ids=input_ids, attention_mask=attention_mask,
                                  output_hidden_states=False).last_hidden_state
        # Project only the masked position of each sequence through the MLM head.
        hidden = hidden.flatten(0, 1).index_select(0, positions)
        hidden = model.vocab_layer_norm(model.activation(model.vocab_transform(hidden)))
        return model.vocab_projector(hidden)


def export(checkpoint, directory):
    model = DistilBertForMaskedLM.from_pretrained(
        checkpoint, local_files_only=True, attn_implementation="eager", weights_only=True,
    ).eval()
    torch.set_num_threads(2)
    directory.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=directory) as temporary:
        staging = Path(temporary)
        graph = staging / "masked-lm.onnx"
        ids = torch.ones((2, 12), dtype=torch.int64)
        with torch.no_grad():
            torch.onnx.export(
                MaskedRows(model), (ids, ids, torch.tensor([1, 14, 15])), graph,
                input_names=["input_ids", "attention_mask", "positions"],
                output_names=["logits"], opset_version=17, dynamo=False,
                dynamic_axes={
                    "input_ids": {0: "batch", 1: "sequence"},
                    "attention_mask": {0: "batch", 1: "sequence"},
                    "positions": {0: "positions"},
                    "logits": {0: "positions"},
                },
            )
        fp16 = staging / "masked-lm-fp16.onnx"
        onnx.save(float16.convert_float_to_float16(onnx.load(graph)), fp16)
        onnx.checker.check_model(fp16)
        shutil.copyfile(checkpoint / "spiece.model", staging / "spiece.model")
        (staging / "spiece.model").replace(directory / "spiece.model")
        fp16.replace(directory / fp16.name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path,
                        default=Path(os.environ["LOCALAPPDATA"]) / "ViME" / "japanese-typo")
    parser.add_argument("--checkpoint", type=Path, help="use a previously downloaded LINE checkpoint")
    args = parser.parse_args()
    if args.checkpoint:
        export(args.checkpoint, args.directory)
    else:
        with tempfile.TemporaryDirectory() as temporary:
            checkpoint = Path(temporary)
            for name in ("config.json", "spiece.model", "pytorch_model.bin"):
                print(f"Downloading {name} from {MODEL}", flush=True)
                urlretrieve(f"https://huggingface.co/{MODEL}/resolve/{REVISION}/{name}",
                            checkpoint / name)
            export(checkpoint, args.directory)
    print(f"Prepared {args.directory}")


if __name__ == "__main__":
    main()
