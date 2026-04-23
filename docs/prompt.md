# Damascus Fold 原型矫正任务（for Codex）

## 0. 你的角色与优先级规则

你要对仓库 `src/` 下的 Rust 代码做**逐文件审计 + 原地矫正**，使其严格符合论文 *Damascus Fold: Proof of Storage-Time with Deterministic Auditing*（以下简称"论文"，用户会随本 prompt 附上 PDF）。

**当以下来源冲突时，严格按此优先级决策**：

1. 论文规范 > 现有代码 > README / agent.md
2. 密码学正确性 > 性能
3. 当发现代码与论文冲突时，**改代码，不要改规范**；不要为了保留"好看的性能数字"而保留任何降级路径

任何以"README 是这么写的"、"之前的版本是这么做的"、"这样改会让性能变差"为理由拒绝修正的行为都是不允许的。

## 1. 论文硬规范（HARD SPEC，不允许降级）

### SPEC-1：代数基础环必须是 `R_q = Z_q[X]/(X^n+1)`

论文 §II-A：`q` 是奇素数，`R_q = Z_q[X]/(X^n+1)`，`n` 是 2 的幂，`R_q^k` 是 rank-k 自由模。

需要建立的类型层次：
- `FieldElement`：`Z_q` 元素（基于 SPEC-7 选定的 `q`）
- `RingElement`：`Z_q[X]/(X^n+1)` 的元素，支持 negacyclic 乘法（`X^n = -1`）
- `ModuleElement<K>`：`[RingElement; K]`

**禁止**：用 Goldilocks `p = 2^64 - 2^32 + 1` 或任何单素数字段冒充 `R_q`；即使加 feature flag 也不允许保留这种路径。

### SPEC-2：承诺必须是 `R_q^k` 的元素，不是标量

论文 §II-C：`com = Σ f_i · g_i + Σ r_i · h_i ∈ R_q^k`，其中 `f_i, r_i ∈ R_q`，`g_i, h_i ∈ R_q^k`，`·` 是环标量乘模向量。

`Commit` 函数签名**必须**返回 `ModuleElement<K>`。

**禁止**：
- 把承诺压成 `u64`、`[u64; N]`、`blake3::Hash` 或单个 `FieldElement`
- 用 hash 截断冒充承诺（hash 只允许出现在 Fiat-Shamir 挑战派生里）

### SPEC-3：文件必须扩展到 `f_0 ∈ R_0^{N_0}`，严禁"固定维度累加"

论文 §IV-B：`N_0 = 2^d`, `n_0 = 2^d`，`d` 随文件大小增长，`f_0` 把整个文件展开到 `N_0 × n_0` 个 `Z_q` 系数。

**必须删除的反模式（README 里已明文存在）**：
- "按 8 字节流式映射并累加到固定 `vector_len * poly_len` 状态"
- 任何 `state[idx % FIXED_LEN] += ...` 风格的模加累加
- 任何"承诺内存占用不随文件大小增长"的预处理路径

**正确做法**：
- 文件字节 → `Z_q` 系数做**单射**打包（每 `⌊log₂ q / 8⌋` 字节映射成一个 `FieldElement`，不丢信息）
- `d` 从文件大小和 `(q, n)` 推导；末端按 0 填充到 `N_0 = n_0 = 2^d`，并在 `stmt` 里记录原始长度
- 若内存不够装下 `f_0`，**报错**或提示文件分片，**不得**退化到累加器

### SPEC-4：生成元 `g_0, h_0` 是 `stmt` 的固定成员

论文 §IV-B：`stmt = (fileID, d, com_0, g_0, h_0)`。`g_0, h_0` 在 `Register` 时**一次性**固化。

允许的优化：`stmt` 只存 `seed`，约定 `g_{0,i} = PRF(seed, "g", i)`, `h_{0,i} = PRF(seed, "h", i)`，**但 Verify 必须沿论文 §IV-B 末尾递归做 generator folding**：

```
g̃_j = g_{j,L} + x_j^{-1} g_{j,R}
g_{j+1} = g̃_{j,even} + y_j^{-1} g̃_{j,odd}
```

最终用 `g_d = (g*), h_d = (h*)` 检查论文式 (4)：`com_d ?= m* · g* + r* · h*`。

**禁止**：Verify 时直接 hash 派生终端 `g*, h*` 而跳过 generator folding 递归；每轮刷新生成元 seed。

### SPEC-5：cross-term 必须是 `R_q^k` 元素

论文：`L_vec_j, R_vec_j, L_poly_j, R_poly_j ∈ R_j^k`。

`struct RoundRecord` 的四个字段每一个都必须是 `ModuleElement<K>`。

**健康检查**：单轮序列化字节数应约为 `4 · K · n_j · ⌈log₂ q⌉ / 8`。典型参数（K=4, n_0=256, q≈2^32）下，第 0 轮 ≈ 4 KB。如果你测出来是 70 B/轮或 280 B/轮，说明没改对。

### SPEC-6：挑战派生

```
x_j = H(ρ_e ‖ "vec"  ‖ fileID ‖ e ‖ j ‖ com_j  ‖ L_vec_j  ‖ R_vec_j )  ∈ F_q*
y_j = H(ρ_e ‖ "poly" ‖ fileID ‖ e ‖ j ‖ com̃_j ‖ L_poly_j ‖ R_poly_j) ∈ F_q*
```

- 域分离标签 `"vec"` / `"poly"` 必须作为独立字段编码
- 输入顺序严格如上
- 若输出 mod q == 0，用 counter 递增重采，直到落在 `F_q*`
- `x_j, y_j` 是 `Z_q` 标量

### SPEC-7：MSIS 参数选择

在 `src/utils/config.rs` 里作为常量固定，并在 `docs/params.md` 里说明理由。三选一（或自选等价强度方案）：

- **方案 A（轻量）**：`q = 8380417`（Dilithium 素数，23 位），`n = 256`，`k = 4`
- **方案 B（中等）**：`q = 2^32 - 2^20 + 1 = 4293918721`，`n = 64`，`k = 8`
- **方案 C（严格）**：参照 LaBRADOR / Greyhound (Fenzi-Moghaddas-Nguyen) 参数表

目标 MSIS 安全等级 λ ≥ 128。**禁止**使用 Goldilocks `2^64 - 2^32 + 1`。

## 2. 已知违规清单（逐条自查 + 汇报 + 修正）

对每一条：(a) 用下方 `rg` 命令定位 → (b) 判断是否违规 → (c) 违规则按 SPEC 重写 → (d) 在最终报告里列出结果（即使未违规也要写 "N/A — 实际代码已符合，证据：`path:line`"）。

| ID | 违规模式 | 定位命令 | 对应 SPEC |
|----|---------|---------|----------|
| V1 | Goldilocks 标量域冒充 R_q | `rg -n "0xFFFFFFFF00000001\|18446744069414584321\|Goldilocks" src/` | SPEC-1, 7 |
| V2 | 固定维度累加器 | `rg -n "wrapping_add\|%\s*(vector_len\|poly_len\|FIXED_)\|state\[.*%" src/utils/ src/protocol/` | SPEC-3 |
| V3 | 承诺返回标量/摘要 | `rg -n "fn commit\|-> FieldElement\|-> \[u64\|-> Hash\|-> u64" src/commitment/` | SPEC-2 |
| V4 | 生成元按轮重派生 / Verify 跳过 generator folding | `rg -n "derive_g\|gen_from_seed\|fn.*generator" src/commitment/ src/protocol/` | SPEC-4 |
| V5 | cross-term 被压缩/截断 | 看 `RoundRecord` 字段类型；打印 `bincode::serialize(&rec).len()` | SPEC-5 |
| V6 | 挑战派生缺域分离/未拒绝 0 | `rg -n "Transcript\|challenge" src/protocol/transcript.rs` | SPEC-6 |
| V7 | NTT 未作用于承诺的环乘法 | `rg -n "ntt\(\|forward_ntt\|inverse_ntt" src/commitment/ src/protocol/` | SPEC-1, 2 |
| V8 | `crates/` 残留旧实现 | `ls crates/ && find crates/ -name '*.rs'` | — |

## 3. 迭代工作流（每 Phase 完成后 `cargo build && cargo test && git commit`）

### Phase 1：代数层重建
- 新 `FieldElement`（`Z_q`，按 SPEC-7）
- 新 `RingElement`（`R_q`，negacyclic）+ 其 `add/sub/mul`
- 基于新 `q` 的 forward/inverse NTT
- 不变式单测：`ntt(intt(a)) == a`、`(X^n) · 1 + 1 ≡ 0 in R_q`、`a·b·b^{-1} == a`
- Commit: `feat(algebra): rewrite R_q ring under SPEC-1, SPEC-7`

### Phase 2：承诺层重建
- 引入 `ModuleElement<K>` 及模上的线性运算
- 重写 `commit` 返回 `ModuleElement<K>`
- 生成元改为 `Register` 一次性固化进 `stmt`
- 同态测试：`commit(a·f + b·f', a·r + b·r') == a·commit(f,r) + b·commit(f',r')`（`a, b ∈ R_q`）
- Commit: `feat(commitment): R_q^k linear commitment under SPEC-2, SPEC-4`

### Phase 3：预处理层重建
- `Prover::initialize`：`d = depth_from_size(file_size, q, n)`，分配 `f_0: Vec<RingElement>` 长度 `N_0 = 2^d`
- 文件 → `FieldElement` 的单射打包；padding 记录原始长度
- 内存不够直接报错，不退化
- 单射测试：两个不同的 1 MB 文件得到不同的 `com_0`
- Commit: `fix(prover): expand file to f_0 ∈ R_0^{N_0} under SPEC-3`

### Phase 4：fold 层重建
- `RoundProve` / `RoundReplay` 按论文 §IV-B 重写，cross-term 是 `ModuleElement<K>`
- Fiat-Shamir 按 SPEC-6 实现，含域分离和零拒绝
- Verify 末尾做 generator folding 递归，末端比对式 (4)
- E2E 测试：honest → accept；任意位翻转 → reject
- Commit: `feat(fold): two-stage fold under SPEC-5, SPEC-6`

### Phase 5：对抗性健全性测试（必须通过）
- **碰撞测试**：构造两个不同 1 MB 文件 A、B，断言 `com_0(A) != com_0(B)`（旧实现在固定累加下会碰撞）
- **篡改测试**：随机翻转任一 micro-block 的一字节，Verify 必须拒绝
- **参数 sweep**：改变 `q / n / K`，所有不变式仍成立
- Commit: `test: adversarial & parameter-sweep suites`

### Phase 6：基准校准
- 重跑 `cargo bench`
- **期望现象**：preprocessing 吞吐从 139 MB/s 掉到 1–10 MB/s 量级。**这是正确信号**，不是退步
- 如果仍然测出 100+ MB/s，**停下来**，Phase 1–3 里一定还留着降级路径
- 更新 README 和 bench 表，删除所有"固定 vector_len * poly_len"表述
- Commit: `bench: recalibrate under honest MSIS parameters`

## 4. 终止条件（必须全部满足）

- [ ] `cargo build --release` 通过
- [ ] `cargo test` 通过（含 Phase 5 对抗测试）
- [ ] `rg "Goldilocks|0xFFFFFFFF00000001"` 在 `src/` 下返回 0 行
- [ ] `rg "FIXED_|state\[.*%" src/utils/ src/protocol/` 返回 0 行
- [ ] `Commitment::commit` 的返回类型是 `ModuleElement<K>` 或语义等价物
- [ ] 单轮 `recj` 序列化字节数 ≥ `4 · K · n_j · ⌈log₂ q⌉ / 8`
- [ ] `docs/params.md` 存在，说明 MSIS 参数选择依据
- [ ] `docs/divergences.md` 列出所有历史违规和修正 commit 哈希
- [ ] README 的"实现说明"段已重写，不再宣称固定维度累加

## 5. 汇报格式

开始工作前，先汇报一份 V1–V8 自查结果表：

```
| ID | 违规？ | 文件:行号          | 简述           |
|----|--------|-------------------|----------------|
| V1 | 是     | src/algebra/...   | ...            |
| V2 | 是     | src/utils/io.rs   | ...            |
| ...|        |                   |                |
```

然后依次推进 Phase 1 → 6，每个 Phase 结束后汇报该 Phase 修改的文件列表和关键 diff 摘要。

**开始工作**。