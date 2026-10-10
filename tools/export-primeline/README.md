# German dictation model

kyrr's German model is [primeline/parakeet-primeline](https://huggingface.co/primeline/parakeet-primeline)
(CC-BY-4.0, a German fine-tune of NVIDIA's parakeet-tdt-0.6b-v3), converted to
ONNX for sherpa-onnx and quantized to int8.

Changes to the original: exported to ONNX (transducer branch only) and the
encoder's MatMul/Gemm weights quantized to int8 per channel.

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
