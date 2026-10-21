# Local changes to hushmic-denoiser v0.10.2

Source: https://github.com/Fovty/HushMic, tag v0.10.2, `crates/hushmic-denoiser`
(MIT OR Apache-2.0; the native runtime under `native/` is by Ceva, Apache-2.0).

## Native engine builds with MSVC

The `native` feature compiled the DPDFNet C runtime with GCC/Clang only.
Changes for the `x86_64-pc-windows-msvc` target, other targets unchanged:

- `build.rs`: MSVC gets `/std:c11` and `/arch:AVX2` for the AVX2/FP16 files
  instead of `-std=gnu11 -ffp-contract=off -fno-lto -mavx2 -mfma -mf16c`.
  MSVC does not contract to FMA under the default `/fp:precise`, which
  matches `-ffp-contract=off`. No `-lm`: the MSVC C runtime has the math
  functions.
- `native/runtime/dpdf_dprnn.c`: CPU detection via `__cpuid`/`_xgetbv`
  instead of `__builtin_cpu_supports` (checks AVX, FMA, F16C, OSXSAVE,
  XCR0 YMM state and AVX2, like the GCC builtin).
- `native/runtime/fp16.c`: scalar FP16 packing via `_mm_cvtps_ph` on a
  128-bit register instead of `_cvtss_sh` (not available in MSVC); same
  rounding mode, identical result.
- `native/runtime/internal.h`, `int8.c`: `DPDF_NOINLINE` macro instead of
  `__attribute__((noinline))`.
- `Cargo.toml`: workspace-inherited fields written out.
- Every modified C file starts with a modification notice (Apache-2.0 §4(b)).
- `native/README.md`: note about the modified files added at the top.
- `tests/` and `examples/` are upstream as-is; they need HushMic's asset
  script and fixtures and are not built or run in dpdf-voice.

## kyrr

- `rustfmt.toml` added (empty settings) so the workspace's `max_width = 120`
  does not reformat the vendored sources.

## kyrr: VNNI kernels

The INT8 matrix kernels in `native/runtime/int8.c` exist in three builds,
picked once per process by CPU (`dpdf_int8_kernel` in `dpdf_dprnn.c`):

- AVX2: the code as it was (the plain build of `int8.c`).
- AVX-VNNI (Intel 12th gen and newer, AMD Zen 5): `build.rs` compiles
  `int8.c` a second time with `DPDF_BUILD_AVXVNNI` and `-mavxvnni`.
- AVX-512 VNNI on 256-bit vectors (Intel 10th/11th gen mobile, AMD Zen 4):
  a third time with `DPDF_BUILD_AVX512VNNI`, `-mavx512f -mavx512vl
  -mavx512vnni` (MSVC: `/arch:AVX512`).

The VNNI builds keep activations unsigned (`a + 127`) and use one
`vpdpbusd` per weight vector instead of abs/sign/maddubs/madd/add. The
integer sums are exactly those of the AVX2 code, so the output is bit for
bit the same (`crates/kyrr-denoise/tests/int8_kernels.rs`). On a Xeon with
both: dpdfnet8 29 % → 20 % of one core, dpdfnet2 11.6 % → 9.1 %.

- `int8.c`: macros for the inner product, activation offset and packing;
  `dpdf_qaffine` and `dpdf_qaffine_pair` dispatch to the VNNI builds;
  `dpdf_qcreate` detects the CPU at load time, off the audio thread.
- `dpdf_dprnn.c`: CPUID/XCR0 detection (`dpdf_int8_kernel`), the
  `DPDF_INT8_KERNEL` environment variable (`avx2`, or `avx512vnni` where
  the CPU has it) to pick another build for comparisons, and
  `dpdf_int8_use` for tests.
- `internal.h`: their declarations.
- `build.rs`, `src/native.rs`, `src/lib.rs` (changes marked "kyrr:"): the
  two extra builds, `int8_kernel()`, `use_int8_kernel()` (tests), and the
  kernel in `Engine`'s text ("native int8 avx-vnni").
