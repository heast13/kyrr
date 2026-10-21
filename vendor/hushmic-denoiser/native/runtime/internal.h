/* Modified 2026 by heast13 for dpdf-voice: MSVC (Windows) support, and for
 * kyrr: the choice of INT8 kernels.
 * See vendor/hushmic-denoiser/PATCHES.md. Original: ceva-ip/DPDFNet, Apache-2.0. */
#ifndef DPDF_INTERNAL_H
#define DPDF_INTERNAL_H
#include "dpdf_dprnn.h"
#include <stdint.h>
#if defined(_MSC_VER) && !defined(__clang__)
#define DPDF_NOINLINE __declspec(noinline)
#else
#define DPDF_NOINLINE __attribute__((noinline))
#endif
typedef void (*dpdf_affine_fn)(const float *, const float *, const float *, float *, int, int, int);
typedef void (*dpdf_gates_fn)(const float *, const float *, const float *, float *, int);
typedef void (*dpdf_norm_fn)(float *,const float *,const float *,const float *,const float *,float,int);
void dpdf_affine_scalar(const float *, const float *, const float *, float *, int, int, int);
void dpdf_gates_scalar(const float *, const float *, const float *, float *, int);
#ifdef DPDF_X86_DISPATCH
typedef struct dpdf_qmatrix dpdf_qmatrix;
dpdf_qmatrix *dpdf_qcreate(const float *, int, int);
void dpdf_qdestroy(dpdf_qmatrix *);
size_t dpdf_qbytes(const dpdf_qmatrix *);
void dpdf_qaffine(const dpdf_qmatrix *, const float *, const float *, float *, int);
void dpdf_qaffine_pair(const dpdf_qmatrix *, const dpdf_qmatrix *, const float *,
                       const float *, const float *, float *, float *, int);
/* kyrr: which build of the INT8 kernels in int8.c runs. Detected once; the
 * DPDF_INT8_KERNEL environment variable (avx2, or avx512vnni where the CPU
 * has it) picks another one for comparisons. dpdf_int8_use is for tests. */
enum { DPDF_INT8_AVX2 = 0, DPDF_INT8_AVXVNNI = 1, DPDF_INT8_AVX512VNNI = 2 };
int dpdf_int8_kernel(void);
int dpdf_int8_use(int kernel);
void dpdf_pack_fp16(const float *, uint16_t *, size_t);
void dpdf_affine_fp16(const float *, const uint16_t *, const float *, float *, int, int, int);
void dpdf_affine_avx2(const float *, const float *, const float *, float *, int, int, int);
void dpdf_gates_avx2(const float *, const float *, const float *, float *, int);
void dpdf_activations_avx2(const float *, float *, float *, size_t);
void dpdf_norm_residual_avx2(float *,const float *,const float *,const float *,const float *,float,int);
#endif
#endif
