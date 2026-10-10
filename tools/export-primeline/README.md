# German dictation model

kyrr's German model is [primeline/parakeet-primeline](https://huggingface.co/primeline/parakeet-primeline)
(CC-BY-4.0, a German fine-tune of NVIDIA's parakeet-tdt-0.6b-v3), converted to
ONNX for sherpa-onnx and quantized to int8.

Changes to the original: exported to ONNX (transducer branch only), the
encoder's MatMul/Gemm weights quantized to int8 per channel, and the int8
weights of encoder and joiner stored as uint8 (see below).

## Rebuild

Python 3.12, CPU only:

```bash
python -m venv venv
venv/bin/pip install torch --index-url https://download.pytorch.org/whl/cpu
venv/bin/pip install "nemo_toolkit[asr]" onnx onnxruntime
curl -L -o primeline.nemo https://huggingface.co/primeline/parakeet-primeline/resolve/main/2_95_WER.nemo
venv/bin/python export.py "$PWD/primeline.nemo" "$PWD/fp32"
venv/bin/python quantize.py fp32 de-primeline
```

## Accuracy

Word error rate on 100 clips of Multilingual LibriSpeech German (test) and
73 clips of LibriSpeech English (validation, clean), lower is better:

| Variant | German | English |
|---|---|---|
| fp32 | 6.92 % | 3.74 % |
| int8, every op, per tensor (sherpa-onnx default) | 7.76 % | 4.35 % |
| int8, MatMul/Gemm per channel (kyrr) | 6.84 % | 4.17 % |

## Why uint8 weights

ONNX Runtime runs `MatMulInteger` with signed int8 weights as U8S8. On x86
CPUs with AVX2 but without VNNI (Intel before Ice Lake/Alder Lake, AMD before
Zen 4) it uses `VPMADDUBSW`, which adds pairs of products saturating in int16.
In the encoder's 217 `MatMulInteger` nodes that overflows and the model
returns nothing or a stray word. With VNNI (`VPDPBUSD`, int32) it works.

`quantize.py` therefore adds 128 to every int8 weight and to its zero point
(`to_u8u8`). `(B - b_zp)` stays the same, so the result is exact, only the
type changes and ONNX Runtime takes the U8U8 kernel, which widens to 16 bit
before multiplying. On 173 clips of the same two test sets every transcript
is identical to the int8 model's. On a CPU with VNNI the U8U8 kernel is about a third slower
(still about 50x real time on a Ryzen 7 9800X3D with 4 threads).

The first upload, `de-primeline`, still had int8 weights; kyrr now uses
`de-primeline-u8`.
