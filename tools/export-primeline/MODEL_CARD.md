---
license: other
license_name: cc-by-4.0-and-nvidia-open-model-license
license_link: LICENSE.md
language:
- de
- en
- fr
- es
- it
- nl
- pl
- pt
base_model:
- primeline/parakeet-primeline
- nvidia/parakeet-tdt-0.6b-v3
- nvidia/parakeet-unified-en-0.6b
tags:
- automatic-speech-recognition
- onnx
- sherpa-onnx
- int8
---

# kyrr-models

Models that [kyrr](https://github.com/heast13/kyrr) downloads for dictation.
The same files are attached to the `models-*` releases of
[heast13/kyrr-assets](https://github.com/heast13/kyrr-assets), a second
mirror. Each folder has its own license, see [LICENSE.md](LICENSE.md).

## de-primeline-u8

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
  decoder and joiner per tensor
- the int8 weights of encoder and joiner stored as uint8 (+128 on weights and
  zero points, exact), so the model also works on x86 CPUs without VNNI;
  nothing else changed

| File | Size | SHA256 |
|---|---|---|
| `de-primeline-u8/encoder.int8.onnx` | 881,596,736 | `49596a2c9c20a224fd087b2c1dc8cdc5dfe3fb8d3ddd4eefb070d81e492c93d8` |
| `de-primeline-u8/decoder.int8.onnx` | 11,845,274 | `6b32c9339900086afd3d282eb40688ae19f3d106281e7731ee27ead1add8b29d` |
| `de-primeline-u8/joiner.int8.onnx` | 6,355,277 | `c84fcf848108531a7309ef65308dc5dc610091ce0fbde134f76bb0f6c42ca530` |
| `de-primeline-u8/tokens.txt` | 93,939 | `d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d` |

## eu-v3-u8

[nvidia/parakeet-tdt-0.6b-v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3)
by NVIDIA, licensed under
[CC-BY-4.0](https://creativecommons.org/licenses/by/4.0/). It knows 25
European languages and picks the right one by itself.

Changes made for kyrr: the same as for `de-primeline-u8` above, nothing else.

| File | Size | SHA256 |
|---|---|---|
| `eu-v3-u8/encoder.int8.onnx` | 881,596,698 | `098defd35fcbbcc5e4a5fa3e7832920cb7686f03c26103d24ae61ea176a62b46` |
| `eu-v3-u8/decoder.int8.onnx` | 11,845,274 | `0222a86147299d4dee90edf95b6d13827edb42ad8b7c9cf0b2582f030937c0b5` |
| `eu-v3-u8/joiner.int8.onnx` | 6,355,277 | `fcc04e8d98f879196bb76015e3d91067ffa8304fb33c6ed8c04a72f53c9c0452` |
| `eu-v3-u8/tokens.txt` | 102,132 | `ba8e4007c65f4bb4358ffe2ecc13d9ccc7a10351151065242b5c3a943e685742` |

## en-unified

[nvidia/parakeet-unified-en-0.6b](https://huggingface.co/nvidia/parakeet-unified-en-0.6b),
Licensed by NVIDIA Corporation under the NVIDIA Open Model License; a copy
of the license is in `en-unified/NVIDIA-Open-Model-License.pdf`
(version of October 24, 2025, from
[nvidia.com](https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/)).

ONNX int8 conversion for sherpa-onnx by the sherpa-onnx project, from
[csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming](https://huggingface.co/csukuangfj2/sherpa-onnx-nemo-parakeet-unified-en-0.6b-int8-non-streaming)
at revision `8c3a10fb13408c7a7054f6898958bf1c64a8d6c7`. Mirrored unchanged,
so kyrr's downloads do not depend on that repository.

| File | Size | SHA256 |
|---|---|---|
| `en-unified/encoder.int8.onnx` | 654,040,552 | `6716910b7a0833997fec7a410494c995d70124001a0e9b66d6370d6aced577e0` |
| `en-unified/decoder.int8.onnx` | 7,257,753 | `a5e223392c90e75f8144cdb5eb95af7625db389e39edef2bd1a9c872b3298fe6` |
| `en-unified/joiner.int8.onnx` | 1,735,860 | `869f43f7d24595c55581ad3bf249a935fb8a71389fbdaa7504b9f46f93140f8a` |
| `en-unified/tokens.txt` | 8,952 | `dc0b4584ab2e4ddbf888425c076c61b736e7356a015250db7d307e6f1a8188ff` |

## de-primeline (superseded)

The first upload, used by kyrr 0.2.0: the same model with signed int8
weights in encoder and joiner. ONNX Runtime gets these wrong on x86 CPUs
without VNNI (Intel before Ice Lake/Alder Lake, AMD before Zen 4). Kept so
the pinned links of older kyrr versions keep working.
