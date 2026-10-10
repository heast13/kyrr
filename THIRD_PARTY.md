# Third-party components

kyrr is licensed under MIT OR Apache-2.0. It builds on the following
components, each under its own license.

| Component | Author | License | Where |
|---|---|---|---|
| DPDFNet models (`dpdfnet8_48khz_hr`, `dpdfnet2_48khz_hr`) and packed weights (`*.weights.f32`) | Ceva ([ceva-ip/DPDFNet](https://github.com/ceva-ip/DPDFNet)) | Apache-2.0 | fetched by `scripts/fetch-assets.py` from the HushMic v0.10.2 release |
| DPDFNet native C runtime and generated model code | Ceva | Apache-2.0 | `vendor/hushmic-denoiser/native/` (see its `LICENSE` and `README.md`) |
| hushmic-denoiser v0.10.2 | Fovty ([HushMic](https://github.com/Fovty/HushMic)) | MIT OR Apache-2.0 | `vendor/hushmic-denoiser/`, patched for MSVC, see `PATCHES.md` |
| ONNX Runtime 1.27.0 | Microsoft ([onnxruntime](https://github.com/microsoft/onnxruntime)) | MIT | `onnxruntime.dll`, Windows fallback engine |
| rubato | HEnquist ([rubato](https://github.com/HEnquist/rubato)) | MIT | resampling to and from 48 kHz |
| sherpa-onnx 1.13.8 (Rust crate, statically linked with ONNX Runtime) | Xiaomi Corp. and contributors ([sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx)) | Apache-2.0 | dictation engine |
| German dictation model | primeLine AI ([primeline/parakeet-primeline](https://huggingface.co/primeline/parakeet-primeline)), based on NVIDIA parakeet-tdt-0.6b-v3 | CC-BY-4.0 | downloaded on first use; converted to ONNX and quantized to int8 by `tools/export-primeline` |
| English dictation model | NVIDIA ([parakeet-unified-en-0.6b](https://huggingface.co/nvidia/parakeet-unified-en-0.6b)), ONNX int8 by the sherpa-onnx project | NVIDIA Open Model License | downloaded on first use from the sherpa-onnx author's Hugging Face mirror |
