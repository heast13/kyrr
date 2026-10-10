# Quantizes the fp32 export to int8 for kyrr: quantize.py <fp32dir> <outdir>
#
# Only the encoder's MatMul/Gemm weights are quantized (int8, per channel);
# its convolutions stay fp32. Decoder and joiner are quantized per tensor.
# On the German test set this matched fp32 accuracy and ran twice as fast as
# quantizing every op per tensor (sherpa-onnx's default), which lost about
# one WER point.
import os
import shutil
import sys

import onnx
from onnxruntime.quantization import QuantType, quantize_dynamic

src, out = sys.argv[1:3]
os.makedirs(out, exist_ok=True)
meta = eval(open(f"{src}/meta.txt").read())

quantize_dynamic(
    model_input=f"{src}/encoder.onnx",
    model_output=f"{out}/encoder.q.onnx",
    weight_type=QuantType.QInt8,
    per_channel=True,
    op_types_to_quantize=["MatMul", "Gemm"],
)
m = onnx.load(f"{out}/encoder.q.onnx")
while len(m.metadata_props):
    m.metadata_props.pop()
for k, v in meta.items():
    p = m.metadata_props.add()
    p.key, p.value = k, str(v)
onnx.save(m, f"{out}/encoder.int8.onnx")
os.remove(f"{out}/encoder.q.onnx")

for n in ("decoder", "joiner"):
    quantize_dynamic(
        model_input=f"{src}/{n}.onnx",
        model_output=f"{out}/{n}.int8.onnx",
        weight_type=QuantType.QInt8,
    )
shutil.copy(f"{src}/tokens.txt", out)
