/* Modified 2026 by heast13 for dpdf-voice: MSVC (Windows) support, and for
 * kyrr: VNNI builds of the integer kernels (see below).
 * See vendor/hushmic-denoiser/PATCHES.md. Original: ceva-ip/DPDFNet, Apache-2.0. */
/* Original AVX2 W8A8 experiment. Weights: per-output symmetric [-127,127].
 * Activations: per-row asymmetric 254-step range, centered at -127.
 * No static calibration, saturating pair sums, or quantized recurrent state.
 */
#include "internal.h"
#include <immintrin.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>

/* kyrr: this file is compiled three times. The plain build is the AVX2 code
 * as it was; it owns the matrices and picks a kernel per call. The builds
 * with DPDF_BUILD_AVXVNNI (VEX, Intel 12th gen and newer) or
 * DPDF_BUILD_AVX512VNNI (EVEX on 256-bit vectors, Intel 10th/11th gen mobile,
 * AMD Zen 4 and newer) only provide dpdf_qaffine and dpdf_qaffine_pair under
 * other names. They store activations unsigned (q = a + 127, 0..254) and sum
 * four products per lane with one vpdpbusd instead of abs/sign/maddubs/madd/
 * add. The integer sums are the same: sum(w*q) - zp*sum(w) equals
 * sum(w*a) + (127-zp)*sum(w), and no step saturates (|sum| < 2^24). So the
 * output is bit for bit that of the AVX2 build. */
#if defined(DPDF_BUILD_AVXVNNI) || defined(DPDF_BUILD_AVX512VNNI)
#define DPDF_INT8_VNNI 1
#define DPDF_ACT_OFFSET 0
#define DPDF_PACK8 _mm_packus_epi16
#define DPDF_ABS(a) (a)
#define DPDF_ONES (void)0
#ifdef DPDF_BUILD_AVXVNNI
#define DPDF_DOT(sum,act,w,a) _mm256_dpbusd_avx_epi32((sum),(act),(w))
#define dpdf_qaffine dpdf_qaffine_avxvnni
#define dpdf_qaffine_pair dpdf_qaffine_pair_avxvnni
#else
#define DPDF_DOT(sum,act,w,a) _mm256_dpbusd_epi32((sum),(act),(w))
#define dpdf_qaffine dpdf_qaffine_avx512vnni
#define dpdf_qaffine_pair dpdf_qaffine_pair_avx512vnni
#endif
#else
#define DPDF_ACT_OFFSET 127
#define DPDF_PACK8 _mm_packs_epi16
#define DPDF_ABS(a) _mm256_abs_epi8(a)
#define DPDF_ONES const __m256i ones=_mm256_set1_epi16(1)
#define DPDF_DOT(sum,act,w,a) _mm256_add_epi32((sum),_mm256_madd_epi16(_mm256_maddubs_epi16((act),_mm256_sign_epi8((w),(a))),ones))
#endif

struct dpdf_qmatrix {
    int k,n;
    int8_t *packed;
    float *scale;
    int32_t *sum;
};
#ifndef DPDF_INT8_VNNI
void dpdf_qdestroy(dpdf_qmatrix *q) {
    if (q) { free(q->packed); free(q->scale); free(q->sum); free(q); }
}
dpdf_qmatrix *dpdf_qcreate(const float *w,int k,int n) {
    (void)dpdf_int8_kernel(); /* kyrr: detect the CPU now, not on the audio thread */
    if (!w || k<=0 || k>512 || k%8 || n<=0 || n%8) return NULL;
    dpdf_qmatrix *q=calloc(1,sizeof(*q)); if (!q) return NULL;
    q->k=k; q->n=n;
    q->packed=malloc((size_t)k*n); q->scale=malloc(n*sizeof(float)); q->sum=calloc(n,sizeof(int32_t));
    if (!q->packed || !q->scale || !q->sum) { dpdf_qdestroy(q); return NULL; }
    for (int c=0;c<n;++c) {
        float maximum=0;
        for (int j=0;j<k;++j) { if (!isfinite(w[j*n+c])) { dpdf_qdestroy(q); return NULL; } maximum=fmaxf(maximum,fabsf(w[j*n+c])); }
        float scale=maximum>0 ? maximum/127.0f : 1.0f; q->scale[c]=scale;
        for (int j=0;j<k;++j) {
            int v=(int)nearbyintf(w[j*n+c]/scale);
            if (v>127) v=127;
            if (v< -127) v= -127;
            /* Each vector contains four K entries for each of eight outputs. */
            q->packed[(c/8)*k*8+(j/4)*32+(c%8)*4+j%4]=(int8_t)v;
            q->sum[c]+=v;
        }
    }
    return q;
}
size_t dpdf_qbytes(const dpdf_qmatrix *q) {
    return q ? sizeof(*q)+(size_t)q->k*q->n+(size_t)q->n*8 : 0;
}
#endif

static void quantize(const float *x,int8_t *out,int k,float *scale,int *zp) {
    __m256 lo=_mm256_setzero_ps(),hi=lo;
    for (int j=0;j<k;j+=8) { __m256 v=_mm256_loadu_ps(x+j); lo=_mm256_min_ps(lo,v); hi=_mm256_max_ps(hi,v); }
    /* Reduce in registers. The scalar reference starts at zero and ignores
     * unordered comparisons; a zero second operand preserves that behavior. */
    lo=_mm256_min_ps(lo,_mm256_setzero_ps());
    hi=_mm256_max_ps(hi,_mm256_setzero_ps());
    __m128 low4=_mm_min_ps(_mm256_castps256_ps128(lo),_mm256_extractf128_ps(lo,1));
    __m128 high4=_mm_max_ps(_mm256_castps256_ps128(hi),_mm256_extractf128_ps(hi,1));
    low4=_mm_min_ps(low4,_mm_movehl_ps(low4,low4));
    high4=_mm_max_ps(high4,_mm_movehl_ps(high4,high4));
    float low=_mm_cvtss_f32(_mm_min_ss(low4,_mm_shuffle_ps(low4,low4,0x55)));
    float high=_mm_cvtss_f32(_mm_max_ss(high4,_mm_shuffle_ps(high4,high4,0x55)));
    if (low==high) { memset(out,127-DPDF_ACT_OFFSET,k); *scale=1; *zp=127; return; }
    *scale=(high-low)/254.0f;
    *zp=(int)nearbyintf(-low / *scale);
    if (*zp<0) *zp=0;
    if (*zp>254) *zp=254;
    __m256 inv=_mm256_set1_ps(1.0f / *scale),z=_mm256_set1_ps((float)*zp);
    for (int j=0;j<k;j+=8) {
        __m256 v=_mm256_add_ps(_mm256_mul_ps(_mm256_loadu_ps(x+j),inv),z);
        __m256i q=_mm256_cvtps_epi32(v);
        q=_mm256_min_epi32(_mm256_set1_epi32(254),_mm256_max_epi32(_mm256_setzero_si256(),q));
        q=_mm256_sub_epi32(q,_mm256_set1_epi32(DPDF_ACT_OFFSET));
        __m128i p=_mm_packs_epi32(_mm256_castsi256_si128(q),_mm256_extracti128_si256(q,1));
        p=DPDF_PACK8(p,p); _mm_storel_epi64((__m128i *)(out+j),p);
    }
}

#ifndef DPDF_DISABLE_QAFFINE_ROW
/* Keep integer reduction separate from quantization/dequantization: inlining
 * their live temporaries caused GCC 12 to spill two accumulators every K step.
 * Short weight lifetimes let all eight accumulators stay in registers. The
 * noinline boundary is intentional; this still uses only the existing AVX2 ISA.
 * Inputs/weights exclude -128, so pair sums fit i16 (2*127*127=32258).
 * K<=512 also keeps the full dot product safely within i32. */
static DPDF_NOINLINE void qdot64(const int8_t *activation,
        const int8_t *packed,int k,int32_t *out) {
    DPDF_ONES;
    __m256i sum0=_mm256_setzero_si256(),sum1=sum0,sum2=sum0,sum3=sum0;
    __m256i sum4=sum0,sum5=sum0,sum6=sum0,sum7=sum0;
    for (int j=0;j<k;j+=4) {
        int32_t bytes; memcpy(&bytes,activation+j,4);
        __m256i a=_mm256_set1_epi32(bytes),absolute=DPDF_ABS(a);
        const int8_t *base=packed+j*8;
        __m256i w0=_mm256_loadu_si256((const __m256i *)(base));
        sum0=DPDF_DOT(sum0,absolute,w0,a);
        __m256i w1=_mm256_loadu_si256((const __m256i *)(base+k*8));
        sum1=DPDF_DOT(sum1,absolute,w1,a);
        __m256i w2=_mm256_loadu_si256((const __m256i *)(base+2*k*8));
        sum2=DPDF_DOT(sum2,absolute,w2,a);
        __m256i w3=_mm256_loadu_si256((const __m256i *)(base+3*k*8));
        sum3=DPDF_DOT(sum3,absolute,w3,a);
        __m256i w4=_mm256_loadu_si256((const __m256i *)(base+4*k*8));
        sum4=DPDF_DOT(sum4,absolute,w4,a);
        __m256i w5=_mm256_loadu_si256((const __m256i *)(base+5*k*8));
        sum5=DPDF_DOT(sum5,absolute,w5,a);
        __m256i w6=_mm256_loadu_si256((const __m256i *)(base+6*k*8));
        sum6=DPDF_DOT(sum6,absolute,w6,a);
        __m256i w7=_mm256_loadu_si256((const __m256i *)(base+7*k*8));
        sum7=DPDF_DOT(sum7,absolute,w7,a);
    }
    _mm256_storeu_si256((__m256i *)(out+0),sum0);
    _mm256_storeu_si256((__m256i *)(out+8),sum1);
    _mm256_storeu_si256((__m256i *)(out+16),sum2);
    _mm256_storeu_si256((__m256i *)(out+24),sum3);
    _mm256_storeu_si256((__m256i *)(out+32),sum4);
    _mm256_storeu_si256((__m256i *)(out+40),sum5);
    _mm256_storeu_si256((__m256i *)(out+48),sum6);
    _mm256_storeu_si256((__m256i *)(out+56),sum7);
}

static void qaffine_row(const dpdf_qmatrix *q,const float *x,const float *bias,float *y) {
    int8_t activation[512]; float activation_scale; int zp;
    const int k=q->k,n=q->n;
    quantize(x,activation,k,&activation_scale,&zp);
    DPDF_ONES;
    int c=0;
    for (;c+63<n;c+=64) {
        int32_t integer_sums[64];
        qdot64(activation,q->packed+c*k,k,integer_sums);
        __m256i sums[8];
        for (int t=0;t<8;++t) sums[t]=_mm256_loadu_si256((const __m256i *)(integer_sums+t*8));
        for (int t=0;t<8;++t) {
            int column=c+t*8;
            __m256i correction=_mm256_mullo_epi32(_mm256_loadu_si256((const __m256i *)(q->sum+column)),_mm256_set1_epi32(DPDF_ACT_OFFSET-zp));
            __m256 sum=_mm256_cvtepi32_ps(_mm256_add_epi32(sums[t],correction));
            __m256 scale=_mm256_mul_ps(_mm256_set1_ps(activation_scale),_mm256_loadu_ps(q->scale+column));
            _mm256_storeu_ps(y+column,_mm256_fmadd_ps(sum,scale,_mm256_loadu_ps(bias+column)));
        }
    }
    for (;c<n;c+=8) {
        __m256i sum=_mm256_setzero_si256();
        for (int j=0;j<k;j+=4) {
            int32_t bytes; memcpy(&bytes,activation+j,4);
            __m256i a=_mm256_set1_epi32(bytes);
            __m256i w=_mm256_loadu_si256((const __m256i *)(q->packed+c*k+j*8));
            sum=DPDF_DOT(sum,DPDF_ABS(a),w,a);
        }
        sum=_mm256_add_epi32(sum,_mm256_mullo_epi32(_mm256_loadu_si256((const __m256i *)(q->sum+c)),_mm256_set1_epi32(DPDF_ACT_OFFSET-zp)));
        __m256 scale=_mm256_mul_ps(_mm256_set1_ps(activation_scale),_mm256_loadu_ps(q->scale+c));
        _mm256_storeu_ps(y+c,_mm256_fmadd_ps(_mm256_cvtepi32_ps(sum),scale,_mm256_loadu_ps(bias+c)));
    }
}
#endif

/* Four rows and two independent 8-column tiles. Keeping the integer loop in
 * its own function leaves room for eight accumulators and shared weights;
 * scale/correction temporaries are only live after the reduction returns. */
static DPDF_NOINLINE void qdot4pair(const int8_t *activation,
        const int8_t *packed0,const int8_t *packed1,int k,__m256i *out) {
    DPDF_ONES;
    __m256i a0=_mm256_setzero_si256(),a1=a0,a2=a0,a3=a0;
    __m256i b0=a0,b1=a0,b2=a0,b3=a0;
    for (int j=0;j<k;j+=4) {
        __m256i w0=_mm256_loadu_si256((const __m256i *)(packed0+j*8));
        __m256i w1=_mm256_loadu_si256((const __m256i *)(packed1+j*8));
#define DPDF_DOT_ROW(row,sa,sb) do { \
        int32_t bytes; memcpy(&bytes,activation+(row)*k+j,4); \
        __m256i v=_mm256_set1_epi32(bytes),absolute=DPDF_ABS(v); \
        sa=DPDF_DOT(sa,absolute,w0,v); \
        sb=DPDF_DOT(sb,absolute,w1,v); \
    } while (0)
        DPDF_DOT_ROW(0,a0,b0); DPDF_DOT_ROW(1,a1,b1);
        DPDF_DOT_ROW(2,a2,b2); DPDF_DOT_ROW(3,a3,b3);
#undef DPDF_DOT_ROW
    }
    out[0]=a0;out[1]=a1;out[2]=a2;out[3]=a3;
    out[4]=b0;out[5]=b1;out[6]=b2;out[7]=b3;
}

static void qaffine_batch_tiled(const dpdf_qmatrix *q,const float *x,const float *bias,float *y,int m) {
    int8_t activation[48*512]; float scales[48]; int zp[48];
    const int k=q->k,n=q->n;
    for (int r=0;r<m;++r) quantize(x+r*k,activation+r*k,k,scales+r,zp+r);
    for (int r=0;r<m;r+=4) for (int c=0;c<n;c+=16) {
        __m256i sums[8];
        qdot4pair(activation+r*k,q->packed+c*k,q->packed+(c+8)*k,k,sums);
        for (int t=0;t<4;++t) for (int tile=0;tile<2;++tile) {
            int col=c+tile*8;
            __m256i correction=_mm256_mullo_epi32(_mm256_loadu_si256((const __m256i *)(q->sum+col)),_mm256_set1_epi32(DPDF_ACT_OFFSET-zp[r+t]));
            __m256 scale=_mm256_mul_ps(_mm256_set1_ps(scales[r+t]),_mm256_loadu_ps(q->scale+col));
            __m256 result=_mm256_fmadd_ps(_mm256_cvtepi32_ps(_mm256_add_epi32(sums[t+tile*4],correction)),scale,_mm256_loadu_ps(bias+col));
            _mm256_storeu_ps(y+(r+t)*n+col,result);
        }
    }
}

static void qaffine_batch(const dpdf_qmatrix *q,const float *x,const float *bias,float *y,int m) {
    if (m%4==0 && q->n%16==0) { qaffine_batch_tiled(q,x,bias,y,m); return; }
    /* Callers split larger batches into at most 48 rows. */
    int8_t activation[48*512]; float scales[48]; int zp[48];
    const int k=q->k,n=q->n;
    for (int r=0;r<m;++r) quantize(x+r*k,activation+r*k,k,scales+r,zp+r);
    DPDF_ONES;
    int r=0;
    for (;r+3<m;r+=4) for (int c=0;c<n;c+=8) {
        __m256i s0=_mm256_setzero_si256(),s1=s0,s2=s0,s3=s0;
        for (int j=0;j<k;j+=4) {
            int32_t v0,v1,v2,v3;
            memcpy(&v0,activation+r*k+j,4); memcpy(&v1,activation+(r+1)*k+j,4);
            memcpy(&v2,activation+(r+2)*k+j,4); memcpy(&v3,activation+(r+3)*k+j,4);
            __m256i a0=_mm256_set1_epi32(v0),a1=_mm256_set1_epi32(v1);
            __m256i a2=_mm256_set1_epi32(v2),a3=_mm256_set1_epi32(v3);
            __m256i w=_mm256_loadu_si256((const __m256i *)(q->packed+c*k+j*8));
            s0=DPDF_DOT(s0,DPDF_ABS(a0),w,a0);
            s1=DPDF_DOT(s1,DPDF_ABS(a1),w,a1);
            s2=DPDF_DOT(s2,DPDF_ABS(a2),w,a2);
            s3=DPDF_DOT(s3,DPDF_ABS(a3),w,a3);
        }
        __m256i sums[4]={s0,s1,s2,s3};
        for (int t=0;t<4;++t) {
            __m256i correction=_mm256_mullo_epi32(_mm256_loadu_si256((const __m256i *)(q->sum+c)),_mm256_set1_epi32(DPDF_ACT_OFFSET-zp[r+t]));
            __m256 scale=_mm256_mul_ps(_mm256_set1_ps(scales[r+t]),_mm256_loadu_ps(q->scale+c));
            __m256 result=_mm256_fmadd_ps(_mm256_cvtepi32_ps(_mm256_add_epi32(sums[t],correction)),scale,_mm256_loadu_ps(bias+c));
            _mm256_storeu_ps(y+(r+t)*n+c,result);
        }
    }
    for (;r<m;++r) {
      int c=0;
      /* Recurrent GRU projections arrive here one row at a time. Walk K once
       * for eight output vectors so each activation load/broadcast is reused.
       * Each accumulator retains the original increasing-K sum order. */
      for (;c+63<n;c+=64) {
        __m256i sum0=_mm256_setzero_si256(),sum1=sum0,sum2=sum0,sum3=sum0;
        __m256i sum4=sum0,sum5=sum0,sum6=sum0,sum7=sum0;
        for (int j=0;j<k;j+=4) {
            int32_t bytes; memcpy(&bytes,activation+r*k+j,4);
            __m256i a=_mm256_set1_epi32(bytes);
            __m256i absolute=DPDF_ABS(a);
            const int8_t *base=q->packed+(c/8)*k*8+j*8;
            __m256i w0=_mm256_loadu_si256((const __m256i *)(base));
            __m256i w1=_mm256_loadu_si256((const __m256i *)(base+k*8));
            __m256i w2=_mm256_loadu_si256((const __m256i *)(base+2*k*8));
            __m256i w3=_mm256_loadu_si256((const __m256i *)(base+3*k*8));
            __m256i w4=_mm256_loadu_si256((const __m256i *)(base+4*k*8));
            __m256i w5=_mm256_loadu_si256((const __m256i *)(base+5*k*8));
            __m256i w6=_mm256_loadu_si256((const __m256i *)(base+6*k*8));
            __m256i w7=_mm256_loadu_si256((const __m256i *)(base+7*k*8));
            /* |a|<=127, |w|<=127 => pair sums <=32258, no i16 saturation.
             * Neither operand contains -128, so sign transfer is exact. */
            sum0=DPDF_DOT(sum0,absolute,w0,a);
            sum1=DPDF_DOT(sum1,absolute,w1,a);
            sum2=DPDF_DOT(sum2,absolute,w2,a);
            sum3=DPDF_DOT(sum3,absolute,w3,a);
            sum4=DPDF_DOT(sum4,absolute,w4,a);
            sum5=DPDF_DOT(sum5,absolute,w5,a);
            sum6=DPDF_DOT(sum6,absolute,w6,a);
            sum7=DPDF_DOT(sum7,absolute,w7,a);
        }
        __m256i sums[8]={sum0,sum1,sum2,sum3,sum4,sum5,sum6,sum7};
        for (int t=0;t<8;++t) {
            int column=c+t*8;
            __m256i correction=_mm256_mullo_epi32(_mm256_loadu_si256((const __m256i *)(q->sum+column)),_mm256_set1_epi32(DPDF_ACT_OFFSET-zp[r]));
            __m256 sum=_mm256_cvtepi32_ps(_mm256_add_epi32(sums[t],correction));
            __m256 scale=_mm256_mul_ps(_mm256_set1_ps(scales[r]),_mm256_loadu_ps(q->scale+column));
            _mm256_storeu_ps(y+r*n+column,_mm256_fmadd_ps(sum,scale,_mm256_loadu_ps(bias+column)));
        }
      }
      for (;c<n;c+=8) {
        __m256i sum=_mm256_setzero_si256();
        for (int j=0;j<k;j+=4) {
            int32_t bytes; memcpy(&bytes,activation+r*k+j,4);
            __m256i a=_mm256_set1_epi32(bytes);
            __m256i w=_mm256_loadu_si256((const __m256i *)(q->packed+c*k+j*8));
            sum=DPDF_DOT(sum,DPDF_ABS(a),w,a);
        }
        __m256i correction=_mm256_mullo_epi32(_mm256_loadu_si256((const __m256i *)(q->sum+c)),_mm256_set1_epi32(DPDF_ACT_OFFSET-zp[r]));
        sum=_mm256_add_epi32(sum,correction);
        __m256 scale=_mm256_mul_ps(_mm256_set1_ps(scales[r]),_mm256_loadu_ps(q->scale+c));
        __m256 result=_mm256_fmadd_ps(_mm256_cvtepi32_ps(sum),scale,_mm256_loadu_ps(bias+c));
        _mm256_storeu_ps(y+r*n+c,result);
      }
    }
}

#ifndef DPDF_INT8_VNNI
void dpdf_qaffine_avxvnni(const dpdf_qmatrix *,const float *,const float *,float *,int);
void dpdf_qaffine_avx512vnni(const dpdf_qmatrix *,const float *,const float *,float *,int);
void dpdf_qaffine_pair_avxvnni(const dpdf_qmatrix *,const dpdf_qmatrix *,const float *,
                               const float *,const float *,float *,float *,int);
void dpdf_qaffine_pair_avx512vnni(const dpdf_qmatrix *,const dpdf_qmatrix *,const float *,
                                  const float *,const float *,float *,float *,int);
#endif

void dpdf_qaffine(const dpdf_qmatrix *q,const float *x,const float *bias,float *y,int m) {
#ifndef DPDF_INT8_VNNI
    switch (dpdf_int8_kernel()) {
    case DPDF_INT8_AVXVNNI: dpdf_qaffine_avxvnni(q,x,bias,y,m); return;
    case DPDF_INT8_AVX512VNNI: dpdf_qaffine_avx512vnni(q,x,bias,y,m); return;
    default: break;
    }
#endif
#ifndef DPDF_DISABLE_QAFFINE_ROW
    if (m==1) qaffine_row(q,x,bias,y);
    else qaffine_batch(q,x,bias,y,m);
#else
    qaffine_batch(q,x,bias,y,m);
#endif
}

/* The bidirectional intra-frequency GRU applies two matrices to exactly the
 * same rows. Quantize once and walk both packed matrices together, retaining
 * the original increasing-K accumulation order for every output. */
void dpdf_qaffine_pair(const dpdf_qmatrix *q0,const dpdf_qmatrix *q1,const float *x,
                       const float *bias0,const float *bias1,float *y0,float *y1,int m) {
#ifndef DPDF_INT8_VNNI
    switch (dpdf_int8_kernel()) {
    case DPDF_INT8_AVXVNNI: dpdf_qaffine_pair_avxvnni(q0,q1,x,bias0,bias1,y0,y1,m); return;
    case DPDF_INT8_AVX512VNNI: dpdf_qaffine_pair_avx512vnni(q0,q1,x,bias0,bias1,y0,y1,m); return;
    default: break;
    }
#endif
    if (q0->k!=q1->k || q0->n!=q1->n || m%4) {
        dpdf_qaffine(q0,x,bias0,y0,m); dpdf_qaffine(q1,x,bias1,y1,m); return;
    }
    int8_t activation[48*512]; float scales[48]; int zp[48];
    const int k=q0->k,n=q0->n;
    for (int r=0;r<m;++r) quantize(x+r*k,activation+r*k,k,scales+r,zp+r);
    for (int r=0;r<m;r+=4) for (int c=0;c<n;c+=8) {
        __m256i sums[8];
        qdot4pair(activation+r*k,q0->packed+c*k,q1->packed+c*k,k,sums);
        const __m256i weight_sum0=_mm256_loadu_si256((const __m256i *)(q0->sum+c));
        const __m256i weight_sum1=_mm256_loadu_si256((const __m256i *)(q1->sum+c));
        const __m256 weight_scale0=_mm256_loadu_ps(q0->scale+c);
        const __m256 weight_scale1=_mm256_loadu_ps(q1->scale+c);
        for (int t=0;t<4;++t) {
            __m256i zero=_mm256_set1_epi32(DPDF_ACT_OFFSET-zp[r+t]);
            __m256 scale=_mm256_set1_ps(scales[r+t]);
            __m256i corrected=_mm256_add_epi32(sums[t],_mm256_mullo_epi32(weight_sum0,zero));
            _mm256_storeu_ps(y0+(r+t)*n+c,_mm256_fmadd_ps(_mm256_cvtepi32_ps(corrected),
                             _mm256_mul_ps(scale,weight_scale0),_mm256_loadu_ps(bias0+c)));
            corrected=_mm256_add_epi32(sums[t+4],_mm256_mullo_epi32(weight_sum1,zero));
            _mm256_storeu_ps(y1+(r+t)*n+c,_mm256_fmadd_ps(_mm256_cvtepi32_ps(corrected),
                             _mm256_mul_ps(scale,weight_scale1),_mm256_loadu_ps(bias1+c)));
        }
    }
}
