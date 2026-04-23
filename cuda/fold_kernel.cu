#include <cuda_runtime.h>
#include <stdint.h>

namespace {
constexpr unsigned int LIMB_BITS = 56;
constexpr uint64_t LIMB_MASK_U64 = (1ULL << LIMB_BITS) - 1;
constexpr uint64_t MSIS_Q_LO = 0xFFFFFFFFFFFFFFB5ULL;
constexpr uint64_t MSIS_Q_HI = 0x0000FFFFFFFFFFFFULL;

#if defined(__SIZEOF_INT128__)
constexpr __uint128_t LIMB_MASK = (((__uint128_t)1) << LIMB_BITS) - 1;
constexpr __uint128_t MASK_112 = (((__uint128_t)1) << 112) - 1;
constexpr __uint128_t PSEUDO_MERSENNE_C = 75;
constexpr __uint128_t MSIS_Q =
    ((__uint128_t)0xFFFFFFFFFFFFULL << 64) | 0xFFFFFFFFFFFFFFB5ULL;

using fp_word = __uint128_t;

__device__ __forceinline__ fp_word fp_load(uint64_t lo, uint64_t hi) {
    return ((__uint128_t)hi << 64) | (__uint128_t)lo;
}

__device__ __forceinline__ void fp_store(uint64_t* lo_out, uint64_t* hi_out, fp_word v) {
    *lo_out = static_cast<uint64_t>(v);
    *hi_out = static_cast<uint64_t>(v >> 64);
}

__device__ __forceinline__ fp_word fp_reduce_pseudo_mersenne(fp_word low, fp_word high) {
    fp_word acc = low + high * PSEUDO_MERSENNE_C;
    const fp_word hi = acc >> 112;
    acc = (acc & MASK_112) + hi * PSEUDO_MERSENNE_C;
    while (acc >= MSIS_Q) {
        acc -= MSIS_Q;
    }
    return acc;
}

__device__ __forceinline__ fp_word fp_add(fp_word a, fp_word b) {
    const fp_word sum = a + b;
    return sum >= MSIS_Q ? sum - MSIS_Q : sum;
}

__device__ __forceinline__ fp_word fp_mul(fp_word a, fp_word b) {
    const fp_word a0 = a & LIMB_MASK;
    const fp_word a1 = a >> LIMB_BITS;
    const fp_word b0 = b & LIMB_MASK;
    const fp_word b1 = b >> LIMB_BITS;

    const fp_word c0 = a0 * b0;
    const fp_word c1 = a0 * b1 + a1 * b0;
    const fp_word c2 = a1 * b1;

    const fp_word c1_lo = c1 & LIMB_MASK;
    const fp_word c1_hi = c1 >> LIMB_BITS;

    const fp_word low_sum = c0 + (c1_lo << LIMB_BITS);
    const fp_word low = low_sum & MASK_112;
    const fp_word carry = low_sum >> 112;
    const fp_word high = c2 + c1_hi + carry;

    return fp_reduce_pseudo_mersenne(low, high);
}
#else
struct fp_word {
    uint64_t lo;
    uint64_t hi;
};

struct wide_word {
    uint64_t lo;
    uint64_t hi;
};

__device__ __forceinline__ fp_word fp_load(uint64_t lo, uint64_t hi) {
    return fp_word{lo, hi & MSIS_Q_HI};
}

__device__ __forceinline__ void fp_store(uint64_t* lo_out, uint64_t* hi_out, fp_word v) {
    *lo_out = v.lo;
    *hi_out = v.hi;
}

__device__ __forceinline__ bool fp_ge_modulus(const fp_word& v) {
    return v.hi > MSIS_Q_HI || (v.hi == MSIS_Q_HI && v.lo >= MSIS_Q_LO);
}

__device__ __forceinline__ fp_word fp_sub_modulus(fp_word v) {
    const uint64_t borrow = v.lo < MSIS_Q_LO;
    v.lo -= MSIS_Q_LO;
    v.hi -= MSIS_Q_HI + borrow;
    return v;
}

__device__ __forceinline__ wide_word wide_add(wide_word a, wide_word b) {
    wide_word out{};
    out.lo = a.lo + b.lo;
    out.hi = a.hi + b.hi + (out.lo < a.lo ? 1ULL : 0ULL);
    return out;
}

__device__ __forceinline__ wide_word mul_u64_wide(uint64_t a, uint64_t b) {
    wide_word out{};
    out.lo = a * b;
    out.hi = __umul64hi(a, b);
    return out;
}

__device__ __forceinline__ uint64_t fp_limb0(const fp_word& v) {
    return v.lo & LIMB_MASK_U64;
}

__device__ __forceinline__ uint64_t fp_limb1(const fp_word& v) {
    return (v.lo >> LIMB_BITS) | (v.hi << 8);
}

__device__ __forceinline__ fp_word fp_from_limbs(uint64_t limb0, uint64_t limb1) {
    return fp_word{
        limb0 | (limb1 << LIMB_BITS),
        limb1 >> 8,
    };
}

__device__ __forceinline__ fp_word fp_add(fp_word a, fp_word b) {
    fp_word out{};
    out.lo = a.lo + b.lo;
    out.hi = a.hi + b.hi + (out.lo < a.lo ? 1ULL : 0ULL);
    if (fp_ge_modulus(out)) {
        out = fp_sub_modulus(out);
    }
    return out;
}

__device__ __forceinline__ fp_word fp_mul(fp_word a, fp_word b) {
    const uint64_t a0 = fp_limb0(a);
    const uint64_t a1 = fp_limb1(a);
    const uint64_t b0 = fp_limb0(b);
    const uint64_t b1 = fp_limb1(b);

    const wide_word c0 = mul_u64_wide(a0, b0);
    const wide_word c1 = wide_add(mul_u64_wide(a0, b1), mul_u64_wide(a1, b0));
    const wide_word c2 = mul_u64_wide(a1, b1);

    const uint64_t p0 = c0.lo & LIMB_MASK_U64;
    const uint64_t c0_hi = (c0.lo >> LIMB_BITS) | (c0.hi << 8);
    const uint64_t c1_lo = c1.lo & LIMB_MASK_U64;
    const uint64_t c1_hi = (c1.lo >> LIMB_BITS) | (c1.hi << 8);
    const uint64_t c2_lo = c2.lo & LIMB_MASK_U64;
    const uint64_t c2_hi = (c2.lo >> LIMB_BITS) | (c2.hi << 8);

    const uint64_t temp1 = c0_hi + c1_lo;
    const uint64_t p1 = temp1 & LIMB_MASK_U64;
    const uint64_t carry1 = temp1 >> LIMB_BITS;

    const uint64_t temp2 = c1_hi + carry1 + c2_lo;
    const uint64_t p2 = temp2 & LIMB_MASK_U64;
    const uint64_t carry2 = temp2 >> LIMB_BITS;
    const uint64_t p3 = c2_hi + carry2;

    uint64_t r0 = p0 + 75ULL * p2;
    uint64_t carry = r0 >> LIMB_BITS;
    r0 &= LIMB_MASK_U64;

    uint64_t r1 = p1 + 75ULL * p3 + carry;
    carry = r1 >> LIMB_BITS;
    r1 &= LIMB_MASK_U64;

    r0 += 75ULL * carry;
    carry = r0 >> LIMB_BITS;
    r0 &= LIMB_MASK_U64;

    r1 += carry;
    carry = r1 >> LIMB_BITS;
    r1 &= LIMB_MASK_U64;

    r0 += 75ULL * carry;
    carry = r0 >> LIMB_BITS;
    r0 &= LIMB_MASK_U64;

    r1 += carry;

    fp_word out = fp_from_limbs(r0, r1);
    if (fp_ge_modulus(out)) {
        out = fp_sub_modulus(out);
    }
    return out;
}
#endif

__global__ void fold_kernel(const uint64_t* left_lo,
                            const uint64_t* left_hi,
                            const uint64_t* right_lo,
                            const uint64_t* right_hi,
                            uint64_t* out_lo,
                            uint64_t* out_hi,
                            size_t len,
                            uint64_t challenge_lo,
                            uint64_t challenge_hi) {
    const size_t idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx < len) {
        const fp_word left = fp_load(left_lo[idx], left_hi[idx]);
        const fp_word right = fp_load(right_lo[idx], right_hi[idx]);
        const fp_word challenge = fp_load(challenge_lo, challenge_hi);
        const fp_word folded = fp_add(left, fp_mul(right, challenge));
        fp_store(&out_lo[idx], &out_hi[idx], folded);
    }
}
} // namespace

extern "C" __declspec(dllexport) int damascus_cuda_fold_batch(
    const uint64_t* left_lo,
    const uint64_t* left_hi,
    const uint64_t* right_lo,
    const uint64_t* right_hi,
    uint64_t* out_lo,
    uint64_t* out_hi,
    size_t len,
    uint64_t challenge_lo,
    uint64_t challenge_hi) {
    if (left_lo == nullptr || left_hi == nullptr || right_lo == nullptr ||
        right_hi == nullptr || out_lo == nullptr || out_hi == nullptr || len == 0) {
        return 1;
    }

    static uint64_t* d_left_lo = nullptr;
    static uint64_t* d_left_hi = nullptr;
    static uint64_t* d_right_lo = nullptr;
    static uint64_t* d_right_hi = nullptr;
    static uint64_t* d_out_lo = nullptr;
    static uint64_t* d_out_hi = nullptr;
    static size_t capacity = 0;

    cudaError_t err = cudaSuccess;

    if (len > capacity) {
        if (d_left_lo != nullptr) {
            cudaFree(d_left_lo);
            d_left_lo = nullptr;
        }
        if (d_left_hi != nullptr) {
            cudaFree(d_left_hi);
            d_left_hi = nullptr;
        }
        if (d_right_lo != nullptr) {
            cudaFree(d_right_lo);
            d_right_lo = nullptr;
        }
        if (d_right_hi != nullptr) {
            cudaFree(d_right_hi);
            d_right_hi = nullptr;
        }
        if (d_out_lo != nullptr) {
            cudaFree(d_out_lo);
            d_out_lo = nullptr;
        }
        if (d_out_hi != nullptr) {
            cudaFree(d_out_hi);
            d_out_hi = nullptr;
        }

        err = cudaMalloc(reinterpret_cast<void**>(&d_left_lo), len * sizeof(uint64_t));
        if (err != cudaSuccess) {
            return 2;
        }
        err = cudaMalloc(reinterpret_cast<void**>(&d_left_hi), len * sizeof(uint64_t));
        if (err != cudaSuccess) {
            return 3;
        }
        err = cudaMalloc(reinterpret_cast<void**>(&d_right_lo), len * sizeof(uint64_t));
        if (err != cudaSuccess) {
            return 4;
        }
        err = cudaMalloc(reinterpret_cast<void**>(&d_right_hi), len * sizeof(uint64_t));
        if (err != cudaSuccess) {
            return 5;
        }
        err = cudaMalloc(reinterpret_cast<void**>(&d_out_lo), len * sizeof(uint64_t));
        if (err != cudaSuccess) {
            return 6;
        }
        err = cudaMalloc(reinterpret_cast<void**>(&d_out_hi), len * sizeof(uint64_t));
        if (err != cudaSuccess) {
            return 7;
        }
        capacity = len;
    }

    err = cudaMemcpy(d_left_lo, left_lo, len * sizeof(uint64_t), cudaMemcpyHostToDevice);
    if (err != cudaSuccess) {
        return 8;
    }
    err = cudaMemcpy(d_left_hi, left_hi, len * sizeof(uint64_t), cudaMemcpyHostToDevice);
    if (err != cudaSuccess) {
        return 9;
    }
    err = cudaMemcpy(d_right_lo, right_lo, len * sizeof(uint64_t), cudaMemcpyHostToDevice);
    if (err != cudaSuccess) {
        return 10;
    }
    err = cudaMemcpy(d_right_hi, right_hi, len * sizeof(uint64_t), cudaMemcpyHostToDevice);
    if (err != cudaSuccess) {
        return 11;
    }

    const int threads = 256;
    const int blocks = static_cast<int>((len + threads - 1) / threads);
    fold_kernel<<<blocks, threads>>>(d_left_lo,
                                     d_left_hi,
                                     d_right_lo,
                                     d_right_hi,
                                     d_out_lo,
                                     d_out_hi,
                                     len,
                                     challenge_lo,
                                     challenge_hi);
    err = cudaGetLastError();
    if (err != cudaSuccess) {
        return 12;
    }
    err = cudaDeviceSynchronize();
    if (err != cudaSuccess) {
        return 13;
    }

    err = cudaMemcpy(out_lo, d_out_lo, len * sizeof(uint64_t), cudaMemcpyDeviceToHost);
    if (err != cudaSuccess) {
        return 14;
    }
    err = cudaMemcpy(out_hi, d_out_hi, len * sizeof(uint64_t), cudaMemcpyDeviceToHost);
    if (err != cudaSuccess) {
        return 15;
    }

    return 0;
}
