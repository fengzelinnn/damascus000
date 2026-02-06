# Damascus-conv Rust Prototype

本仓库已按 `agent.md` 重构为单核心库工程，目标是实现 Damascus-conv（Damascus-2D/Tensor）协议原型，并支持：

- Module-SIS 风格承诺
- 双重折叠：Vector Folding + Odd-Even Poly Folding
- Fiat-Shamir Transcript（Blake3）
- NTT 可开关的多项式乘法
- 大文件 mmap + 流式映射到固定内存状态
- Criterion 基准 + CSV/Markdown 报告输出

## 目录结构

```text
.
├── Cargo.toml
├── benches/
│   └── protocol_bench.rs
├── examples/
│   └── full_flow.rs
└── src/
    ├── lib.rs
    ├── algebra/
    │   ├── field.rs
    │   ├── ntt.rs
    │   └── poly.rs
    ├── commitment/
    │   ├── hasher.rs
    │   └── sis.rs
    ├── protocol/
    │   ├── prover.rs
    │   ├── verifier.rs
    │   └── transcript.rs
    └── utils/
        ├── config.rs
        └── io.rs
```

## 快速开始

### 1) 编译

```powershell
cargo build
```

### 2) 运行端到端示例

```powershell
cargo run --example full_flow -- .\sample.txt
```

不传文件路径时会自动生成 `target/full_flow_input.bin`。

## NTT 开关

NTT 通过运行时配置 `RuntimeConfig::ntt_enabled` 控制：

- `true`: 使用 NTT 卷积（规模较大时自动走 NTT）
- `false`: 回退到朴素乘法

示例中可通过环境变量切换：

```powershell
$env:DAMASCUS_NTT="1"
cargo run --example full_flow -- .\sample.txt

$env:DAMASCUS_NTT="0"
cargo run --example full_flow -- .\sample.txt
```

## 测试

```powershell
cargo test
```

## 基准测试

```powershell
cargo bench --bench protocol_bench
```

基准会覆盖以下文件规模并测试 NTT ON/OFF：

- 100 MB
- 500 MB
- 1 GB
- 2 GB
- 4 GB

输出报告位置：

- `target/bench-reports/protocol_metrics.csv`
- `target/bench-reports/protocol_metrics.md`

表头格式：

| File Size | Mode (NTT) | Preprocessing (s) | Vec Fold (ms) | Poly Fold (ms) | Verify (us) | Cross-Term Size (Bytes) |
| --- | --- | --- | --- | --- | --- | --- |

## 实现说明

- 有限域：Goldilocks prime field
- 承诺：按 seed 派生生成元，按需计算，不预生成全量大参数
- 预处理：`memmap2` 映射文件，按 8 字节流式映射并累加到固定 `vector_len * poly_len` 状态，避免 GB 文件直接膨胀到超大 `Vec<FieldElement>`
- 验证：`DamascusVerifier::update_commitment` 仅使用 micro-block 中的常数大小对象更新承诺
