#include <cuda_runtime.h>
#include <stdint.h>

namespace {
constexpr uint64_t MODULUS = 18446744069414584321ull;
constexpr uint64_t EPSILON = 4294967295ull; // 2^32 - 1

__device__ __forceinline__ uint64_t add_fp(uint64_t a, uint64_t b) {
    uint64_t c = a + b;
    if (c < a || c >= MODULUS) {
        c -= MODULUS;
    }
    return c;
}

__device__ __forceinline__ uint64_t mul_fp(uint64_t a, uint64_t b) {
    uint64_t lo = a * b;
    uint64_t hi = __umul64hi(a, b);

    uint64_t x_hi_hi = hi >> 32;
    uint64_t x_hi_lo = hi & EPSILON;

    uint64_t t0 = lo - x_hi_hi;
    if (lo < x_hi_hi) {
        t0 -= EPSILON;
    }

    uint64_t t1 = x_hi_lo * EPSILON;
    uint64_t t2 = t0 + t1;
    if (t2 < t1) {
        t2 += EPSILON;
    }

    if (t2 >= MODULUS) {
        t2 -= MODULUS;
    }
    return t2;
}

__global__ void fold_kernel(const uint64_t* left,
                            const uint64_t* right,
                            uint64_t* out,
                            size_t len,
                            uint64_t challenge) {
    size_t idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx < len) {
        uint64_t scaled = mul_fp(right[idx], challenge);
        out[idx] = add_fp(left[idx], scaled);
    }
}
} // namespace

extern "C" __declspec(dllexport) int damascus_cuda_fold_batch(
    const uint64_t* left,
    const uint64_t* right,
    uint64_t* out,
    size_t len,
    uint64_t challenge) {
    if (left == nullptr || right == nullptr || out == nullptr || len == 0) {
        return 1;
    }

    static uint64_t* d_left = nullptr;
    static uint64_t* d_right = nullptr;
    static uint64_t* d_out = nullptr;
    static size_t capacity = 0;

    int rc = 0;
    cudaError_t err = cudaSuccess;

    if (len > capacity) {
        if (d_left != nullptr) {
            cudaFree(d_left);
            d_left = nullptr;
        }
        if (d_right != nullptr) {
            cudaFree(d_right);
            d_right = nullptr;
        }
        if (d_out != nullptr) {
            cudaFree(d_out);
            d_out = nullptr;
        }

        err = cudaMalloc(reinterpret_cast<void**>(&d_left), len * sizeof(uint64_t));
        if (err != cudaSuccess) {
            return 2;
        }
        err = cudaMalloc(reinterpret_cast<void**>(&d_right), len * sizeof(uint64_t));
        if (err != cudaSuccess) {
            return 3;
        }
        err = cudaMalloc(reinterpret_cast<void**>(&d_out), len * sizeof(uint64_t));
        if (err != cudaSuccess) {
            return 4;
        }
        capacity = len;
    }

    err = cudaMemcpy(d_left, left, len * sizeof(uint64_t), cudaMemcpyHostToDevice);
    if (err != cudaSuccess) {
        return 5;
    }
    err = cudaMemcpy(d_right, right, len * sizeof(uint64_t), cudaMemcpyHostToDevice);
    if (err != cudaSuccess) {
        return 6;
    }

    const int threads = 256;
    const int blocks = static_cast<int>((len + threads - 1) / threads);
    fold_kernel<<<blocks, threads>>>(d_left, d_right, d_out, len, challenge);
    err = cudaGetLastError();
    if (err != cudaSuccess) {
        return 7;
    }
    err = cudaDeviceSynchronize();
    if (err != cudaSuccess) {
        return 9;
    }

    err = cudaMemcpy(out, d_out, len * sizeof(uint64_t), cudaMemcpyDeviceToHost);
    if (err != cudaSuccess) {
        return 8;
    }

    return rc;
}
