#include <cuda_runtime.h>
#include <stdint.h>

namespace {
constexpr size_t BATCH_PRIMES = 8;
constexpr int THREADS_PER_BLOCK = 256;

__device__ __forceinline__ uint64_t mod_add(uint64_t lhs, uint64_t rhs, uint64_t modulus) {
    const uint64_t sum = lhs + rhs;
    return sum >= modulus ? sum - modulus : sum;
}

__device__ __forceinline__ uint64_t mod_sub(uint64_t lhs, uint64_t rhs, uint64_t modulus) {
    return lhs >= rhs ? lhs - rhs : modulus - (rhs - lhs);
}

__device__ __forceinline__ uint64_t mod_mul(uint64_t lhs, uint64_t rhs, uint64_t modulus) {
    return (lhs * rhs) % modulus;
}

__device__ __forceinline__ uint64_t mod_pow(uint64_t base, size_t exp, uint64_t modulus) {
    uint64_t result = 1;
    while (exp > 0) {
        if ((exp & 1U) != 0) {
            result = mod_mul(result, base, modulus);
        }
        base = mod_mul(base, base, modulus);
        exp >>= 1;
    }
    return result;
}

__device__ __forceinline__ size_t stage_len_from_root(uint64_t wlen, uint64_t modulus) {
    size_t len = 1;
    uint64_t current = wlen % modulus;
    do {
        len <<= 1;
        current = mod_mul(current, current, modulus);
    } while (current != 1);
    return len;
}

__global__ void bit_reverse_kernel(uint64_t* data, size_t n, unsigned int shift) {
    const size_t idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx >= n) {
        return;
    }

    const size_t rev =
        static_cast<size_t>(__brevll(static_cast<unsigned long long>(idx)) >> shift);
    if (idx < rev) {
        const uint64_t tmp = data[idx];
        data[idx] = data[rev];
        data[rev] = tmp;
    }
}

__global__ void scale_kernel(uint64_t* data, size_t n, uint64_t factor, uint64_t modulus) {
    const size_t idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx < n) {
        data[idx] = mod_mul(data[idx], factor, modulus);
    }
}

unsigned int log2_size(size_t n) {
    unsigned int log_n = 0;
    while (n > 1) {
        n >>= 1;
        ++log_n;
    }
    return log_n;
}

int blocks_for(size_t count) {
    return static_cast<int>((count + THREADS_PER_BLOCK - 1) / THREADS_PER_BLOCK);
}
} // namespace

__global__ void ntt_stage_kernel(uint64_t* data, size_t n, uint64_t wlen, uint64_t modulus) {
    const size_t idx = blockIdx.x * blockDim.x + threadIdx.x;
    const size_t butterflies = n >> 1;
    if (idx >= butterflies) {
        return;
    }

    const size_t stage_len = stage_len_from_root(wlen, modulus);
    const size_t half = stage_len >> 1;
    const size_t group = idx / half;
    const size_t offset = idx % half;
    const size_t left = group * stage_len + offset;
    const size_t right = left + half;
    const uint64_t twiddle = mod_pow(wlen, offset, modulus);
    const uint64_t t = mod_mul(data[right], twiddle, modulus);
    const uint64_t u = data[left];
    data[left] = mod_add(u, t, modulus);
    data[right] = mod_sub(u, t, modulus);
}

__global__ void pointwise_mul_kernel(uint64_t* a, const uint64_t* b, size_t n, uint64_t mod) {
    const size_t idx = blockIdx.x * blockDim.x + threadIdx.x;
    if (idx < n) {
        a[idx] = mod_mul(a[idx], b[idx], mod);
    }
}

extern "C" __declspec(dllexport) int damascus_cuda_ntt_batch(
    const uint64_t* host_a,
    const uint64_t* host_b,
    uint64_t* host_out,
    size_t n,
    const uint64_t* primes,
    const uint64_t* stage_roots,
    const uint64_t* inv_roots,
    const uint64_t* inv_sizes) {
    if (host_a == nullptr || host_b == nullptr || host_out == nullptr || primes == nullptr ||
        stage_roots == nullptr || inv_roots == nullptr || inv_sizes == nullptr || n == 0 ||
        (n & (n - 1)) != 0) {
        return 1;
    }

    const unsigned int log_n = log2_size(n);
    const unsigned int bitrev_shift = 64U - log_n;
    const size_t total_values = BATCH_PRIMES * n;
    const int value_blocks = blocks_for(n);
    const int butterfly_blocks = blocks_for(n >> 1);

    uint64_t* d_a = nullptr;
    uint64_t* d_b = nullptr;
    uint64_t* d_out = nullptr;
    cudaStream_t streams[BATCH_PRIMES]{};
    bool stream_created[BATCH_PRIMES]{};
    int rc = 0;

    cudaError_t err =
        cudaMalloc(reinterpret_cast<void**>(&d_a), total_values * sizeof(uint64_t));
    if (err != cudaSuccess) {
        rc = 2;
        goto cleanup;
    }

    err = cudaMalloc(reinterpret_cast<void**>(&d_b), total_values * sizeof(uint64_t));
    if (err != cudaSuccess) {
        rc = 3;
        goto cleanup;
    }

    err = cudaMalloc(reinterpret_cast<void**>(&d_out), total_values * sizeof(uint64_t));
    if (err != cudaSuccess) {
        rc = 4;
        goto cleanup;
    }

    for (size_t prime_idx = 0; prime_idx < BATCH_PRIMES; ++prime_idx) {
        err = cudaStreamCreate(&streams[prime_idx]);
        if (err != cudaSuccess) {
            rc = 5;
            goto cleanup;
        }
        stream_created[prime_idx] = true;
    }

    for (size_t prime_idx = 0; prime_idx < BATCH_PRIMES; ++prime_idx) {
        const size_t offset = prime_idx * n;
        cudaStream_t stream = streams[prime_idx];

        err = cudaMemcpyAsync(
            d_a + offset, host_a + offset, n * sizeof(uint64_t), cudaMemcpyHostToDevice, stream);
        if (err != cudaSuccess) {
            rc = 6;
            goto cleanup;
        }

        err = cudaMemcpyAsync(
            d_b + offset, host_b + offset, n * sizeof(uint64_t), cudaMemcpyHostToDevice, stream);
        if (err != cudaSuccess) {
            rc = 7;
            goto cleanup;
        }

        uint64_t* a_slice = d_a + offset;
        uint64_t* b_slice = d_b + offset;
        const uint64_t modulus = primes[prime_idx];
        const uint64_t* forward_roots = stage_roots + prime_idx * log_n;
        const uint64_t* inverse_roots = inv_roots + prime_idx * log_n;

        bit_reverse_kernel<<<value_blocks, THREADS_PER_BLOCK, 0, stream>>>(
            a_slice, n, bitrev_shift);
        err = cudaGetLastError();
        if (err != cudaSuccess) {
            rc = 8;
            goto cleanup;
        }

        bit_reverse_kernel<<<value_blocks, THREADS_PER_BLOCK, 0, stream>>>(
            b_slice, n, bitrev_shift);
        err = cudaGetLastError();
        if (err != cudaSuccess) {
            rc = 9;
            goto cleanup;
        }

        for (unsigned int stage = 0; stage < log_n; ++stage) {
            ntt_stage_kernel<<<butterfly_blocks, THREADS_PER_BLOCK, 0, stream>>>(
                a_slice, n, forward_roots[stage], modulus);
            err = cudaGetLastError();
            if (err != cudaSuccess) {
                rc = 10;
                goto cleanup;
            }

            ntt_stage_kernel<<<butterfly_blocks, THREADS_PER_BLOCK, 0, stream>>>(
                b_slice, n, forward_roots[stage], modulus);
            err = cudaGetLastError();
            if (err != cudaSuccess) {
                rc = 11;
                goto cleanup;
            }
        }

        pointwise_mul_kernel<<<value_blocks, THREADS_PER_BLOCK, 0, stream>>>(
            a_slice, b_slice, n, modulus);
        err = cudaGetLastError();
        if (err != cudaSuccess) {
            rc = 12;
            goto cleanup;
        }

        bit_reverse_kernel<<<value_blocks, THREADS_PER_BLOCK, 0, stream>>>(
            a_slice, n, bitrev_shift);
        err = cudaGetLastError();
        if (err != cudaSuccess) {
            rc = 13;
            goto cleanup;
        }

        for (unsigned int stage = 0; stage < log_n; ++stage) {
            ntt_stage_kernel<<<butterfly_blocks, THREADS_PER_BLOCK, 0, stream>>>(
                a_slice, n, inverse_roots[stage], modulus);
            err = cudaGetLastError();
            if (err != cudaSuccess) {
                rc = 14;
                goto cleanup;
            }
        }

        scale_kernel<<<value_blocks, THREADS_PER_BLOCK, 0, stream>>>(
            a_slice, n, inv_sizes[prime_idx], modulus);
        err = cudaGetLastError();
        if (err != cudaSuccess) {
            rc = 15;
            goto cleanup;
        }

        err = cudaMemcpyAsync(
            d_out + offset, a_slice, n * sizeof(uint64_t), cudaMemcpyDeviceToDevice, stream);
        if (err != cudaSuccess) {
            rc = 16;
            goto cleanup;
        }

        err = cudaMemcpyAsync(
            host_out + offset, d_out + offset, n * sizeof(uint64_t), cudaMemcpyDeviceToHost, stream);
        if (err != cudaSuccess) {
            rc = 17;
            goto cleanup;
        }
    }

    for (size_t prime_idx = 0; prime_idx < BATCH_PRIMES; ++prime_idx) {
        err = cudaStreamSynchronize(streams[prime_idx]);
        if (err != cudaSuccess) {
            rc = 18;
            goto cleanup;
        }
    }

cleanup:
    for (size_t prime_idx = 0; prime_idx < BATCH_PRIMES; ++prime_idx) {
        if (stream_created[prime_idx]) {
            cudaStreamDestroy(streams[prime_idx]);
        }
    }
    if (d_a != nullptr) {
        cudaFree(d_a);
    }
    if (d_b != nullptr) {
        cudaFree(d_b);
    }
    if (d_out != nullptr) {
        cudaFree(d_out);
    }
    return rc;
}
