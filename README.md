# Damascus（原型）使用手册

本仓库是一个 Rust workspace，用于实现/验证 **Damascus 协议原型**里的若干核心组件（当前重点在 `damascus-conv` 的两阶段折叠与端到端仿真），并提供可重复的 CLI 运行与基准测试入口。

> 说明：这是工程原型而非安全审计/形式化证明后的实现；部分模块（如 `damascus-twist`）仍处于占位状态。

## 1. 环境要求

- Rust：需要支持 **Edition 2024**（建议 Rust `1.85+`）。本仓库在 `rustc 1.89.0` 环境下可构建。
- OS：Windows / Linux / macOS 均可；下面示例以 **PowerShell** 为主。

## 2. 项目结构（workspace 成员）

工作区配置见 `Cargo.toml`，成员为 `crates/damascus-*`，根包提供 `damascus` 二进制（`src/main.rs`）。

- `damascus`（根二进制）：统一入口，支持 `sim` 与轻量 `bench`（见下文 `damascus run --flow ...`）。
- `crates/damascus-conv`：卷积协议（两阶段折叠：vector folding + odd/even polynomial folding）。
- `crates/damascus-ring`：多项式与模环/模元运算（`Poly`/`ModuleElem`）。
- `crates/damascus-crypto`：挑战值派生与确定性 RNG（`blake3` + 规范编码）。
- `crates/damascus-types`：基础类型（`FileId`/`Epoch`/`Round`）与规范编码（bincode 固定选项）。
- `crates/damascus-sim`：端到端 epoch 仿真（生成 witness、执行 prover rounds、验证端 public update + final verify）。
- `crates/damascus-chain`：内存链（按 `(file_id, epoch, round)` 存取 transcript）。
- `crates/damascus-commit`：线性承诺接口与一个 Module-SIS 风格承诺键实现。
- `crates/damascus-cli`：另一个更“conv 聚焦”的 CLI（与根二进制功能重叠，但参数默认值不同）。
- `crates/damascus-bench`：Criterion 基准（`cargo bench`）。
- `crates/damascus-twist`：Twist 协议占位（当前 `commit()` 返回 `NotImplemented`）。

另外：`agent.md` 里包含更偏设计/协议对齐的笔记。

## 3. 构建与帮助信息

在仓库根目录执行：

```powershell
cargo build
cargo run -p damascus -- --help
cargo run -p damascus -- run --help
```

做性能对比/跑基准时建议使用 `--release`：

```powershell
cargo run -p damascus --release -- run --flow bench-conv-round --round 0 --iters 200
```

日志级别可用 `RUST_LOG` 控制，例如：

```powershell
$env:RUST_LOG="info"
cargo run -p damascus -- run --flow sim-epoch --file .\some.bin
```

## 4. 使用手册：`damascus`（根二进制）

### 4.1 打印协议参数（用于记录/复现实验）

**操作：**

```powershell
cargo run -p damascus -- params
```

**效果：**

- 输出 `ConvParams` 的 JSON（包含 `q/n0/n_rounds/k/seed_generators`）。
- 适合把一组实验参数直接保存到日志/文件里，保证后续复现实验时参数一致。

可自定义参数（注意 `n0` 必须等于 `2^rounds`）：

```powershell
cargo run -p damascus -- params --q 998244353 --n0 256 --rounds 8 --k 2 --seed 0000...0000
```

### 4.2 运行端到端 epoch 仿真：`--flow sim-epoch`

**目的：** 从文件 bytes 派生 witness，执行 `n_rounds` 轮 prover 折叠，随后在验证端进行：

1) 常数时间 `public_update_round` 重放 transcript 更新公开状态；  
2) `final_verify` 检查最终 opening 与 transcript 一致性。

**操作：**

1) 准备一个输入文件（任意二进制/文本都可以）：

```powershell
"hello damascus" | Out-File -FilePath .\sample.txt -Encoding utf8 -NoNewline
```

2) 运行仿真：

```powershell
cargo run -p damascus --release -- run --flow sim-epoch --file .\sample.txt --epoch 0
```

**效果（输出含义）：**

- `file_id=...`：`blake3(file_bytes)` 的 32 字节十六进制（见 `damascus-sim::file_id_from_bytes`）。
- `rounds=...`：本次执行的 rounds 数（应等于 `--rounds`）。
- `opening_mu0`/`opening_rho0`：最终 opening 多项式的常数项（用于快速 sanity check）。

如果你想把“重复执行后的平均耗时”作为粗略性能指标（不如 Criterion 严谨，但方便快速对比参数/代码改动）：

```powershell
cargo run -p damascus --release -- run --flow sim-epoch --file .\sample.txt --repeat 10
```

### 4.3 微基准：单轮 prover round 的耗时：`--flow bench-conv-round`

**目的：** 只 benchmark `ConvProver::round(file_id, epoch, j, ...)` 某一轮 `j` 的执行时间，用于快速定位热点（例如 stage1 vs stage2 的改动对性能的影响）。

**操作：**

```powershell
cargo run -p damascus --release -- run --flow bench-conv-round --round 0 --iters 200
```

可通过这些参数控制实验可重复性（同一组参数/seed 应产生稳定输出）：

- `--witness-seed <64hex>`：生成随机 witness 的种子（32 字节 hex）。
- `--file-id <64hex>`：用于挑战值派生的 `FileId`（32 字节 hex）。
- `--epoch <u64>`：同样会进入挑战派生输入。

**效果：**

- 输出 `ns_per_iter=...`，用于对比不同代码版本/不同参数下的 round 性能。

> 注意：`q` 若不是素数，`x/y` 的模逆可能不存在，导致运行时报错。建议使用默认的 `998244353`（常用 NTT 素数）。

## 5. 使用手册：`damascus-cli`（crate 内另一个 CLI）

这个 CLI 更聚焦于 conv 参数与单次运行，子命令结构与默认参数和根二进制不同。

查看帮助：

```powershell
cargo run -p damascus-cli -- --help
```

生成参数 JSON：

```powershell
cargo run -p damascus-cli -- params gen --q 998244353 --n0 8 --rounds 3 --k 2
```

跑一次 epoch 仿真（内部调用 `damascus_sim::run_epoch`）：

```powershell
cargo run -p damascus-cli -- run --file .\sample.txt --epoch 0
```

## 6. 测试操作（做什么 → 达成什么效果）

### 6.1 单元测试：验证协议不变量/基本正确性

**操作：**

```powershell
cargo test -p damascus-conv
```

**效果：**

- 运行 `damascus-conv` 中的测试（例如 `stage1_and_stage2_invariants_hold`），用于验证：
  - prover 更新后的公开状态 `C` 与 verifier 的 `public_update_round` 计算一致；
  - 折叠后的承诺不变量满足（重新 `commit` 能得到相同的 `C`）；
  - 篡改 transcript 的交叉项后，`final_verify` 会拒绝。

### 6.2 全工作区测试：确保各 crate 能一起编译/链接

**操作：**

```powershell
cargo test --workspace
```

**效果：**

- 确保 workspace 内所有 crate 一起通过编译与测试（即使某些 crate 当前没有测试，也能验证依赖关系未被破坏）。

### 6.3 格式化与静态检查（可选，但推荐用于开发）

**操作：**

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

**效果：**

- `fmt`：统一代码风格，减少无关 diff。
- `clippy`：提前发现潜在 bug/低效写法；`-D warnings` 用于把警告当错误处理，保证质量门槛一致。

## 7. 基准测试（Benchmark）

### 7.1 Criterion 基准（更稳健的统计）

**操作：**

```powershell
cargo bench -p damascus-bench --bench conv_round
```

**效果：**

- 运行 Criterion 基准 `conv_round_j0`，输出统计结果（均值、方差、回归检测等），适合做严谨的性能对比。

### 7.2 CLI 内置微基准（更快的迭代反馈）

**操作：** 见上文 `--flow bench-conv-round`。

**效果：**

- 快速得到 `ns_per_iter`，便于在改代码的过程中频繁对比。

## 8. 参数与复现建议（避免“跑不动/跑不通”）

- 约束：`n0` 必须是 2 的幂，且满足 `n0 == 2^n_rounds`（代码会校验）。
- 建议：`q` 使用素数（默认 `998244353`），避免模逆不存在导致失败。
- 复现：把你跑实验时的 `params` JSON、`file`、`epoch`、`file-id`、`witness-seed` 一并记录，后续可以完全复现同一条执行路径与输出。
