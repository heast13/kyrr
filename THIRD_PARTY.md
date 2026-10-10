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
