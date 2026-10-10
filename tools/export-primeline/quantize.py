# Quantizes the fp32 export to int8 for kyrr: quantize.py <fp32dir> <outdir>
#
# Only the encoder's MatMul/Gemm weights are quantized (int8, per channel);
# its convolutions stay fp32. Decoder and joiner are quantized per tensor.
# On the German test set this matched fp32 accuracy and ran twice as fast as
# quantizing every op per tensor (sherpa-onnx's default), which lost about
# one WER point.
#
# The int8 weights of encoder and joiner are then stored as uint8 (to_u8u8).
# ONNX Runtime runs signed weights as U8S8 with VPMADDUBSW on x86 CPUs that
# have AVX2 but no VNNI (Intel before Ice Lake/Alder Lake, AMD before Zen 4),
# which saturates in int16: the model then hears nothing. U8U8 widens to
# 16 bit first and cannot saturate. The shift is exact, so the WER stays.
import os
import shutil
import sys

import numpy as np
import onnx
from onnx import numpy_helper
from onnxruntime.quantization import QuantType, quantize_dynamic


def to_u8u8(path):
    """Rewrites MatMulInteger weights from int8 to uint8 in place: +128 on
    the weights and on their zero points, so (B - b_zp) stays the same."""
    m = onnx.load(path)
    inits = {t.name: t for t in m.graph.initializer}
    targets = set()
    for n in m.graph.node:
        if n.op_type == "MatMulInteger":
            if len(n.input) < 4 or n.input[3] not in inits:
                sys.exit(f"{n.name}: weight zero point missing")
            targets.update(i for i in (n.input[1], n.input[3]) if i in inits)
    for n in m.graph.node:
        for i, name in enumerate(n.input):
            if name in targets and not (n.op_type == "MatMulInteger" and i in (1, 3)):
                sys.exit(f"{name} is also used by {n.op_type}; not safe to convert")
    for name in targets:
        t = inits[name]
        if t.data_type == onnx.TensorProto.INT8:
            u8 = (numpy_helper.to_array(t).astype(np.int16) + 128).astype(np.uint8)
            t.CopyFrom(numpy_helper.from_array(u8, name))
    onnx.save(m, path)

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
for n in ("encoder", "joiner"):
    to_u8u8(f"{out}/{n}.int8.onnx")
shutil.copy(f"{src}/tokens.txt", out)
