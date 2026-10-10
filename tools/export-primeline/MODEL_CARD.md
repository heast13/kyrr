---
license: cc-by-4.0
language:
- de
- en
base_model: primeline/parakeet-primeline
tags:
- automatic-speech-recognition
- onnx
- sherpa-onnx
- int8
---

# kyrr-models

Models that [kyrr](https://github.com/heast13/kyrr) downloads for dictation.

## de-primeline

[primeline/parakeet-primeline](https://huggingface.co/primeline/parakeet-primeline)
by primeLine AI, a German fine-tune of NVIDIA
[parakeet-tdt-0.6b-v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3),
both licensed under [CC-BY-4.0](https://creativecommons.org/licenses/by/4.0/).

Changes made for kyrr, with
[`tools/export-primeline`](https://github.com/heast13/kyrr/tree/main/tools/export-primeline):

- exported from the NeMo checkpoint to ONNX as encoder, decoder and joiner
  for [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx)
- quantized to int8 with ONNX Runtime's dynamic quantization: the
  encoder's MatMul/Gemm weights per channel (its convolutions stay fp32),
  decoder and joiner per tensor; nothing else changed

| File | Size | SHA256 |
|---|---|---|
| `de-primeline/encoder.int8.onnx` | 881,596,736 | `e2d1a42b442e7499ecdc9647a358e75540db9691503d430c7a6bc4263b6e0321` |
| `de-primeline/decoder.int8.onnx` | 11,845,274 | `6b32c9339900086afd3d282eb40688ae19f3d106281e7731ee27ead1add8b29d` |
| `de-primeline/joiner.int8.onnx` | 6,355,277 | `ff5ec87a02959bcc792f7012931665de36cb2f03963e0d8def0d6193b5c6159f` |
| `de-primeline/tokens.txt` | 93,939 | `d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d` |
