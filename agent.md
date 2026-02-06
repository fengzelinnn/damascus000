# Damascus-conv Protocol: Rust Implementation Specification

## 1. 项目概述 (Project Overview)
本项目旨在实现 *Damascus: Stateless Proof of Storage-time* 论文中提出的 **Damascus-conv** (即 Damascus-2D/Tensor) 协议。
目标是构建一个用于验证性测试的工业级原型，重点测试在大文件（GB级别）下的预处理效率、折叠速度、通信开销（Micro-block大小）及验证延迟。

**核心需求：**
- 语言：Rust (Edition 2021)
- 核心算法：Module-SIS 承诺，双重折叠（Vector Folding + Odd-Even Poly Folding）。
- 性能特性：支持 NTT 加速（可开关），并行计算支持。
- 测试目标：生成详细的性能与文件大小的关系报告。

## 2. 架构设计 (Architecture)

### 2.1 目录结构
```text
damascus-core/
├── Cargo.toml
├── benches/             # 基准测试脚本 (Criterion)
│   └── protocol_bench.rs
├── src/
│   ├── lib.rs
│   ├── algebra/         # 代数基础层
│   │   ├── field.rs     # 有限域定义 (Goldilocks field 或类似适合NTT的素域)
│   │   ├── poly.rs      # 多项式运算 (R_q 环运算)
│   │   └── ntt.rs       # 数论变换实现 (Iterative/Recursive)
│   ├── commitment/      # 密码学原语
│   │   ├── sis.rs       # Module-SIS 承诺实现
│   │   └── hasher.rs    # Random Oracle (基于 Blake3 或 Poseidon)
│   ├── protocol/        # 协议核心逻辑
│   │   ├── prover.rs    # Prover: 预处理与生成 Micro-blocks
│   │   ├── verifier.rs  # Verifier: 验证 Micro-blocks
│   │   └── transcript.rs# Fiat-Shamir 变换与挑战生成
│   └── utils/           # 工具函数
│       ├── io.rs        # 大文件 mmap 读取与分块
│       └── config.rs    # 全局配置 (NTT开关, 参数设置)
└── examples/            # 完整的运行示例
    └── full_flow.rs

```

### 2.2 关键依赖 (Dependencies)

在 `Cargo.toml` 中应包含：

* **数学与并行**: `rayon` (并行迭代), `rand` (随机数), `num-bigint`/`num-traits`.
* **序列化**: `serde`, `bincode` (用于测量 Micro-block 大小).
* **哈希**: `blake3` (高性能哈希，用于 Random Oracle).
* **文件处理**: `memmap2` (大文件零拷贝读取).
* **基准测试**: `criterion` (统计学严谨的性能测试).
* **错误处理**: `thiserror`, `anyhow`.

---

## 3. 详细模块规范 (Module Specifications)

### 3.1 代数层 (`src/algebra`)

**目标**：实现环  的算术运算。

1. **Field (`field.rs`)**:
* 选择一个支持 NTT 的 64 位素数 （例如 Goldilocks Field  或 Baby Bear），以确保现代 CPU 上的高性能。
* 实现基本的 `Add`, `Sub`, `Mul`, `Inv`。


2. **NTT (`ntt.rs`)**:
* 实现 `forward_ntt` 和 `inverse_ntt`。
* **Requirement**: 提供一个 `NTT_ENABLED` 的配置标志（可以是编译时 feature 或运行时 Config 结构体）。
* 如果 `NTT_ENABLED = false`，多项式乘法回退到  的朴素算法。


3. **Polynomial (`poly.rs`)**:
* 结构体 `Poly` 包含系数向量 `Vec<Fp>`。
* 实现多项式加法、标量乘法。
* 实现 **Odd-Even Decomposition**：
* 。


* 实现多项式乘法（基于 NTT 状态自动选择算法）。



### 3.2 承诺层 (`src/commitment`)

**目标**：实现 Module-SIS 及其同态特性。

1. **Params**:
* 生成公共参数 。这些是大维度的多项式向量。
* 为了节省内存，不要预生成所有 ，应基于 Seed 使用伪随机生成器（PRNG）按需生成或流式生成。


2. **Commitment Calculation**:
* 公式：。
* **优化**：使用 `Rayon` 的 `par_iter` 并行计算点积。



### 3.3 协议层 (`src/protocol`)

这是核心逻辑所在，对应论文中的 Algorithm 1 和 Algorithm 2。

#### 3.3.1 Prover (`prover.rs`)

需实现一个 `DamascusProver` 结构体，维护当前轮次的状态 。

**功能函数**：

1. **`initialize(file_path: &Path, params: SystemParams) -> Self`**:
* 使用 `memmap2` 映射文件。
* 将二进制数据切片并映射到  的系数上。
* 填充 (Padding) 到 。


2. **`fold_round(&mut self, round_idx: usize) -> RoundOutput`**:
* **Stage 1 (Vector Folding)**:
* 将  切分为 。
* 计算交叉项 （并行计算）。
* 调用 `Transcript` 获取挑战 。
* 更新状态： 等。


* **Stage 2 (Poly Folding)**:
* 对所有多项式执行奇偶分解。
* 计算交叉项 。
* 调用 `Transcript` 获取挑战 。
* 更新状态： 等。


* 返回 `MicroBlocks`。



#### 3.3.2 Verifier (`verifier.rs`)

需实现 `DamascusVerifier`，仅维护承诺 。

**功能函数**：

1. **`update_commitment(micro_blocks: &MicroBlocks) -> Result<()>`**:
* 执行常数时间的承诺更新（仅涉及少量群运算，不涉及长向量）。
* 重建挑战 。
* 验证逻辑参照论文公式 (2) 和 (3)。



---

## 4. 基准测试规范 (Benchmarking Specification)

使用 `criterion` crate 编写 `benches/protocol_bench.rs`。必须包含以下测试组：

### 4.1 测试变量

* **File Sizes**: 100 MB, 500 MB, 1 GB, 2 GB, 4 GB.
* **NTT Status**: On / Off.

### 4.2 测量指标 (Metrics)

1. **Preprocessing Time**:
* 从磁盘读取文件 -> 映射到有限域 -> 生成初始承诺  的耗时。
* *注：这是 I/O 密集型和计算密集型的混合。*


2. **Vector Folding Time (Stage 1)**:
* 计算  和折叠向量  的耗时。
* 主要受向量维度  影响。


3. **Poly Folding Time (Stage 2)**:
* 计算  和折叠多项式系数的耗时。
* 主要受多项式度数  影响 (NTT 性能关键点)。


4. **Total Round Time**:
* 完成一个完整  轮次的耗时。


5. **Micro-block Size**:
* 序列化后的 `L_vec, R_vec, L_poly, R_poly` 字节大小。
* 验证其是否为常数级（Constant Size）。


6. **Verification Time**:
* Verifier 更新承诺  的耗时。
* 理论上应极快且与文件大小无关（O(1)）。



### 4.3 输出格式要求

测试结束后，需要生成 CSV 或 Markdown 表格，格式如下：

| File Size | Mode (NTT) | Preprocessing (s) | Vec Fold (ms) | Poly Fold (ms) | Verify (us) | Cross-Term Size (Bytes) |
| --- | --- | --- | --- | --- | --- | --- |
| 1 GB | ON | ... | ... | ... | ... | ... |
| 1 GB | OFF | ... | ... | ... | ... | ... |

---

## 5. 实现提示 (Implementation Tips for Code Agent)

1. **内存管理**：对于 4GB 文件，如果直接加载到 `Vec<FieldElement>`，内存占用会膨胀（假设每个元素 8 字节，膨胀率可能达到 8 倍甚至更多）。
* **策略**：对于过大的文件，不要一次性将所有  加载到内存。使用流式处理（Iterators）结合 `Rayon`。


2. **NTT 优化**：预计算 Omega powers (Twiddle factors) 并缓存，避免重复计算。
3. **安全性**：对于验证性测试，可以使用固定种子生成公共参数 ，无需通过复杂的 Trusted Setup，但要确保生成的参数对于 Prover 是不可预测的（在协议逻辑上）。
4. **Fiat-Shamir**：使用 `blake3` 实现 Transcript 时，确保将所有上下文（Layer index, previous commitment）都吸收进哈希状态。

## 6. 任务指令 (Instruction to Code Agent)

请基于上述规范生成 Rust 代码。你的响应应包含：

1. 完整的 `Cargo.toml`。
2. 所有核心模块的源代码（尤其是 `algebra`, `commitment`, `protocol`）。
3. `benches` 下的性能测试代码。
4. 一个 `README.md`，说明如何运行测试以及如何开启/关闭 NTT。

开始编码时，请优先保证 `lib.rs` 的结构清晰，并确保数学运算的准确性优于微小的性能优化，待正确性验证后再利用 Rayon 和 unsafe 指针操作进行极速优化。

```

```