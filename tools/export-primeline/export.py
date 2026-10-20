# Exports primeline/parakeet-primeline (.nemo) to the sherpa-onnx layout in fp32.
# Based on sherpa-onnx scripts/nemo/parakeet-tdt-0.6b-v3/export_onnx.py
# (Copyright 2025 Xiaomi Corp., Apache-2.0).
# Usage: export.py <model.nemo> <outdir> [name]
# name: "primeline" (default) or "parakeet-v3", only for the metadata.
import os, sys
import onnx, torch
import nemo.collections.asr as nemo_asr

def add_meta(src, dst, meta, external=None):
    m = onnx.load(src)
    while len(m.metadata_props): m.metadata_props.pop()
    for k, v in meta.items():
        p = m.metadata_props.add(); p.key = k; p.value = str(v)
    if external:
        onnx.save(m, dst, save_as_external_data=True, all_tensors_to_one_file=True, location=external)
    else:
        onnx.save(m, dst)

@torch.no_grad()
def main():
    nemo_path, out = sys.argv[1], sys.argv[2]
    source = {
        "primeline": ("NeMo / primeline", "https://huggingface.co/primeline/parakeet-primeline (fine-tuned parakeet-tdt-0.6b-v3)"),
        "parakeet-v3": ("NeMo / NVIDIA", "https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3"),
    }[sys.argv[3] if len(sys.argv) > 3 else "primeline"]
    os.makedirs(out, exist_ok=True); os.chdir(out)
    asr = nemo_asr.models.ASRModel.restore_from(restore_path=nemo_path, map_location="cpu")
    asr.eval()
    with open("tokens.txt", "w", encoding="utf-8") as f:
        for i, s in enumerate(asr.joint.vocabulary): f.write(f"{s} {i}\n")
        f.write(f"<blk> {i+1}\n")
    asr.encoder.export("encoder.raw.onnx")
    asr.decoder.export("decoder.onnx")
    asr.joint.export("joiner.onnx")
    nt = asr.cfg.preprocessor.normalize
    meta = {
        "vocab_size": asr.decoder.vocab_size, "normalize_type": "" if nt == "NA" else nt,
        "pred_rnn_layers": asr.decoder.pred_rnn_layers, "pred_hidden": asr.decoder.pred_hidden,
        "subsampling_factor": 8, "model_type": "EncDecRNNTBPEModel", "version": "2",
        "model_author": source[0],
        # sherpa-onnx detects TDT models by "tdt" in the url.
        "url": source[1],
        "comment": "Only the transducer branch is exported", "feat_dim": asr.cfg.preprocessor.features,
    }
    print("meta", meta)
    add_meta("encoder.raw.onnx", "encoder.onnx", meta, external="encoder.weights")
    os.remove("encoder.raw.onnx")
    for f in os.listdir("."):
        if f not in ("encoder.onnx", "encoder.weights", "decoder.onnx", "joiner.onnx", "tokens.txt"):
            os.remove(f)  # per-tensor files from the raw export
    open("meta.txt", "w").write(repr(meta))

main()
