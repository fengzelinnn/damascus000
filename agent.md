# agent.md

## 0. 目标与边界

目标：实现 Damascus 的模块化原型，用于真实使用场景的性能评估。要求：

* 各模块可单独编译、单独压测（micro-bench / stress）。
* 模块间可集成测试（end-to-end + 篡改负例）。
* Rust workspace，多 crate 拆分，接口稳定、可替换实现（例如多项式乘法后端）。
* 密码学底层尽量用成熟 crate；对 “Module-SIS 安全性” 不在原型阶段做形式化证明，但要把承诺与折叠链的计算严格按论文的等式与更新规则实现（以便性能与工程形态可信）。

协议锚点（实现必须对齐）：

* Damascus-conv 的承诺形式与不变量：C(j)=Σ f_i^(j)·g_i^(j)+Σ r_i^(j)·h_i^(j) 以及两阶段折叠保持不变量。
* 每轮固定做两次折叠：先 vector folding（压缩 N），再 odd-even polynomial folding（压缩 n）。
* Stage-1/Stage-2 的四个 cross-term：L_vec,R_vec,L_poly,R_poly 的定义与承诺更新公式。
* challenge 派生：xj 用域分离标签 “vec”，yj 用 “poly”，且需要保证非 0（若为 0 则加计数器重试）。
* 公共验证者常数时间更新：只解析 micro-block 并重算 challenge 后更新 C。
* 最终打开：R 轮后收敛到标量 (μ,ρ)，验证 C(R)=μ·g*+ρ·h*。
* warm-up Damascus-twist：Pedersen 类承诺 + 每轮输出 (Lj,Rj) 两个群元素；challenge 来自 VRF 与前序 micro-block 哈希；最终用乘法/多指数检查收敛。

实现注意：论文里写 Zq 与 F*q 混用；为了工程原型可选 “q 为素数” 以把系数域当作有限域 Fq，从而 xj,yj 可逆自然成立（仍要实现非 0 重试逻辑）。xj,yj 属于 F*q 这一点在描述里明确出现。

---

## 1. Workspace 拆分与目录结构

建议 Rust workspace（cargo workspaces）：

* crates/

  * damascus-types/

    * 协议通用类型：FileId, Epoch, Round, Params, 序列化格式、hash 输入规范
  * damascus-crypto/

    * hash/random oracle、签名、VRF、域元素抽样、hash-to-point/hash-to-field
  * damascus-ring/

    * 多项式/环 Rq^(j) 运算（加法、标量乘、negacyclic 乘、odd-even 分解、fold）
    * 可插拔乘法后端：naive / ntt / (可选) rayon 并行
  * damascus-commit/

    * “线性承诺”统一接口（论文 II-B1 抽象接口）：SetupCommit/Commit/ProveLinear/VerifyLinear
    * 原型里 ProveLinear/VerifyLinear 对 conv 不一定做 ZK 证明（因为主方案 conv 在文稿里采用 cross-term 链进行公开重放更新），但接口保留，便于后续替换为真实 lattice ZK 组件
  * damascus-twist/

    * twist 协议：向量承诺、每轮 Twist 计算、micro-block 结构、final resolve 检查
  * damascus-conv/

    * conv 协议：多项式向量编码、Module-SIS 线性承诺计算、两阶段折叠、micro-block、public update、final opening
  * damascus-chain/

    * 最小链环境：随机性源、micro-block 池、committee 聚合器（macro-block 模拟）、可重放 transcript
  * damascus-sim/

    * 端到端仿真：多个 storage node + 多 verifier + committee；可配置网络延迟/丢包（可先用内存通道）
  * damascus-cli/

    * 命令行：生成参数、注册文件、跑若干 epoch、导出统计
  * damascus-bench/

    * Criterion benchmark：模块级与集成级
  * damascus-fuzz/

    * cargo-fuzz：micro-block 解析与验证 fuzz，避免 panic/DoS

---

## 2. 依赖选型（成熟 crate 优先）

基础：

* 序列化：serde + bincode 或 postcard（建议 bincode + 固定配置，确保跨平台字节一致）
* hash：blake3（快且稳定）或 sha2
* 日志：tracing + tracing-subscriber
* CLI：clap
* 并行：rayon
* bench：criterion
* property test：proptest
* fuzz：cargo-fuzz

VRF / 签名：

* schnorrkel（sr25519，提供 VRF + 签名生态，工程成熟）

  * 如果你更想贴近 ed25519：也可 ed25519-dalek + 单独 VRF crate，但 “一个库同时搞定 VRF+签名” 用 schnorrkel 最省心

椭圆曲线群（twist）：

* curve25519-dalek（Ristretto 点，成熟）
* 若要 IPA/Bulletproof 方向：bulletproofs（可选，不是本次必须）

有限域/FFT（conv，NTT 后端）：

* arkworks：ark-ff, ark-poly, ark-serialize

  * 用 ark-poly 的 Radix2EvaluationDomain 做 FFT/NTT，成熟可靠
  * 需要自定义素数域（例如 998244353）可用 ark_ff::MontConfig 定义

---

## 3. damascus-types：核心数据结构与字节规范

关键点：所有 challenge 派生必须字节确定性；建议统一一个 “CanonicalEncoding”：

* FileId：32 字节（blake3(file_bytes) 或 用户给定）
* Epoch u64, Round u32
* DomainTag：固定 ASCII 字符串，严格使用 "vec" 与 "poly"（conv）
* Commitment / ModuleElem / Poly / MicroBlock 的序列化：固定 endian，小心 Rust 默认 Vec 序列化差异（用 bincode 的固定选项）

建议类型（伪代码）：

```rust
pub type FileId = [u8; 32];

#[derive(Clone, Copy)]
pub struct Epoch(pub u64);
#[derive(Clone, Copy)]
pub struct Round(pub u32);

#[derive(Clone)]
pub struct ProtocolId(pub [u8; 16]); // optional，区分网络/参数集

#[derive(Clone)]
pub struct TranscriptId {
    pub file_id: FileId,
    pub epoch: Epoch,
    pub round: Round,
}

pub enum DomainTag { Vec, Poly, Twist /*...*/ }
```

---

## 4. damascus-crypto：RandomOracle、VRF、挑战派生

### 4.1 RandomOracle trait

conv 要求：

* xj = H("vec", fileID, e, j, C(j), L_vec, R_vec)，且 xj ∈ F*q，若为 0 则 counter++ 重试。 
* yj = H("poly", fileID, e, j, C(j+1/2), L_poly, R_poly)，同样非 0。

建议接口：

```rust
pub trait RandomOracle<F: FieldElem> {
    fn challenge_nonzero(
        &self,
        tag: DomainTag,
        input: &[u8],
    ) -> F; // 内部实现 counter 重试直到非零
}
```

### 4.2 VRF（twist 与 committee）

twist 里 challenge 来自 VRF + 前序 micro-block hash。
chain 模块也需要 VRF 选 committee（你的 pdf 里 role 描述有 committee via VRF）。

建议：

* schnorrkel：VRF 输出作为 seed，再喂给 RandomOracle 变成标量 xj
* 原型链：把 “上一 micro-block hash” 拼入 input，保证可重放

---

## 5. damascus-ring：Rq^(j) 环、多项式、odd-even、乘法后端

conv 的环递推：R_q^(0)=Zq[X]/(X^{n0}+1)，R_q^(j+1)=Zq[Y]/(Y^{n_{j+1}}+1)，Y=X^2，且 nj+1=nj/2。

### 5.1 多项式表示

* Poly：系数向量 coeffs: Vec<Fq>，长度固定为 n_j（2 的幂）
* Rq 的乘法：negacyclic mod (X^n + 1)
* ModuleElem：长度 k 的向量，每个坐标是 Poly

### 5.2 odd-even decomposition

按论文：z(X)=z_even(Y)+X z_odd(Y)，Y=X^2。

实现：

* even[t] = coeff[2t]
* odd[t] = coeff[2t+1]

### 5.3 乘法后端（可插拔）

定义 trait：

```rust
pub trait PolyMulBackend<F: FieldElem> {
    fn mul_negacyclic(a: &Poly<F>, b: &Poly<F>) -> Poly<F>;
}
```

实现两版：

* Naive：O(n^2)，小参数测试与 correctness
* Ntt：用 ark-poly 的 FFT 域做长度 2n 变换实现 negacyclic（性能压测主力）
* 可选 rayon：在 cross-term 求和与多项式乘里做并行

---

## 6. damascus-conv：主协议模块（全量挑战、可重放、常数验证更新）

这一部分是你要压测的核心。

### 6.1 参数与状态（与论文对齐）

* 文件编码为多项式向量 F(0)=(f1^(0),...,f_{N0}^(0))，N0=2^R，n0=2^R，padding 同深度 R。
* 每轮状态：F(j), R(j), G(j), H(j), C(j)。
* 初始承诺：C(0)=Σ f_i^(0)·g_i^(0)+Σ r_i^(0)·h_i^(0)，其中 “·” 是环元素对 module 向量的坐标乘。
* 不变量：对每个 j 都保持上述形式。

建议 Rust 结构：

```rust
pub struct ConvParams<F> {
    pub q_modulus: String,     // 仅用于记录
    pub n0: usize,
    pub n_rounds: usize,       // R
    pub k: usize,              // module dimension
    pub seed_generators: [u8; 32],
}

pub struct ConvPublicState<F> {
    pub g: Vec<ModuleElem<F>>, // G(j): length Nj
    pub h: Vec<ModuleElem<F>>, // H(j): length Nj
    pub c: ModuleElem<F>,      // C(j) in (Rq)^k
}

pub struct ConvWitness<F> {
    pub f: Vec<Poly<F>>,       // F(j): length Nj
    pub r: Vec<Poly<F>>,       // R(j): length Nj
}
```

生成器派生：G(0),H(0) “derived from a public seed”。
实现上：用 hash(seed || "G" || i || coord) -> Poly 系数，确保可重放。

### 6.2 Micro-block 结构（conv）

论文明确每轮上链两个 micro-block：MB-vec(e,j) 与 MB-poly(e,j)，各携带常数大小 cross-term。

```rust
pub struct MbVec<F> {
    pub file_id: FileId,
    pub epoch: u64,
    pub round: u32,
    pub l_vec: ModuleElem<F>,
    pub r_vec: ModuleElem<F>,
    pub sig: Vec<u8>, // 可选：storage node 签名，链上身份绑定
}

pub struct MbPoly<F> {
    pub file_id: FileId,
    pub epoch: u64,
    pub round: u32,
    pub l_poly: ModuleElem<F>,
    pub r_poly: ModuleElem<F>,
    pub sig: Vec<u8>,
}
```

### 6.3 Prover：每轮两阶段折叠（严格照算法）

按 Algorithm 1（Prover），步骤要一模一样：先 Stage 1 vector folding，再 Stage 2 odd-even folding。

Stage 1（vector folding）：

* split 成左右半：F_L,F_R，R_L,R_R，G_L,G_R，H_L,H_R（长度 Nj/2）
* 计算 cross-term：

  * L_vec = Σ f_L,i·g_R,i + Σ r_L,i·h_R,i
  * R_vec = Σ f_R,i·g_L,i + Σ r_R,i·h_L,i 
* 派生 xj：H("vec", fileID,e,j,C(j),L_vec,R_vec)，确保非 0。 
* 更新折叠对象（注意 G/H 用 x^{-1}）：

  * F(j+1/2)=F_L + x F_R
  * R(j+1/2)=R_L + x R_R
  * G(j+1/2)=G_L + x^{-1} G_R
  * H(j+1/2)=H_L + x^{-1} H_R
* 更新承诺：C(j+1/2)=C(j)+x^{-1} L_vec + x R_vec 

Stage 2（odd-even polynomial folding）：

* 对任意 z(X) 写成 zeven(Y)+X zodd(Y)，Y=X^2；对 F,R,G,H 的每个 entry 做 odd-even 分解。
* 计算 cross-term：

  * L_poly = Σ f_even,i · g_odd,i + Σ r_even,i · h_odd,i
  * R_poly = Σ f_odd,i · g_even,i + Σ r_odd,i · h_even,i 
* 派生 yj：H("poly", fileID,e,j,C(j+1/2),L_poly,R_poly)，非 0。 
* 更新折叠对象：

  * F(j+1)=F_even + y F_odd
  * R(j+1)=R_even + y R_odd
  * G(j+1)=G_even + y^{-1} G_odd
  * H(j+1)=H_even + y^{-1} H_odd
* 更新承诺：C(j+1)=C(j+1/2)+y^{-1} L_poly + y R_poly 
* 输出 MB-vec 与 MB-poly。

实现接口：

```rust
pub struct ConvProver<'a, F, RO> { /* params, ro, signing key */ }

impl<'a, F, RO> ConvProver<'a, F, RO> {
    pub fn round(
        &self,
        file_id: FileId,
        epoch: u64,
        j: u32,
        pub_state: &mut ConvPublicState<F>,
        wit: &mut ConvWitness<F>,
    ) -> (MbVec<F>, MbPoly<F>);
}
```

### 6.4 Public verifier update：常数时间更新 C

按 Algorithm 2：只靠 micro-block 重算 xj,yj 并更新 C。

```rust
pub fn public_update_round<F, RO>(
    ro: &RO,
    pub_state: &mut ConvPublicState<F>,
    mb_vec: &MbVec<F>,
    mb_poly: &MbPoly<F>,
);
```

压测点：这个函数必须做到 per-round O(k * poly_ops) 的常数更新，不触碰文件大小 N,n 的原始规模。

### 6.5 Final opening：收敛标量检查

论文：R 轮后 NR=1 且 nR=1，F(R)=(μ)，R(R)=(ρ)；g*,h* 通过对 (G(0),H(0)) 重放同样 folds 得到；验证 C(R)= μ·g* + ρ·h*。

实现建议：

* prover 输出 opening: (mu, rho)（都在 R_q^(R) 上，nR=1 等价于常量系数）
* verifier 端提供 replay_fold_generators(params, transcript)->(g_star,h_star)

  * transcript 必须包含每轮的 (xj,yj)，而 (xj,yj) 可由 micro-block + C 的更新序列重算得到（可重放性）。
* 然后做一次 module 等式比较

---

## 7. damascus-twist：warm-up 协议模块（用于对照与基线）

你主要压测 conv，但 twist 作为工程基线很有用（曲线群 + 简单折叠）。

关键流程（按文稿）：

* Commit：C = u^r ∏ g_i^{f_i}。
* 每轮：

  * challenge xj 来自 VRF + 前序 micro-block hash。
  * split f,g 为左右半，计算 cross-terms：

    * Lj = g_R^{f_L} · u^{rL}
    * Rj = g_L^{f_R} · u^{rR} 
  * 本地 fold：f' = xj fL + xj^{-1} fR。
* Verify：检查群元素合法性与 VRF 合规。
* Resolve（final）：收集 {(Lj,Rj)} 与 {xj} 做最终多指数等式检查。

实现上建议用 curve25519-dalek 的 RistrettoPoint + Scalar：

* g_i 与 u 用 hash-to-point（Ristretto）从公共 seed 派生，保证 CRS 无陷门（与文稿 “来自前序区块 hash” 的想法一致）。
* cross-term 计算与承诺计算用 multiscalar multiplication（dalek 提供）

twist 模块接口与 conv 类似：

* prover_round -> microblock
* verifier_round_check
* final_resolve_check

---

## 8. damascus-chain：最小链与 committee（可重放 transcript 的载体）

目的：把 “协议计算” 放进一个可复现实验环境，模拟真实使用场景的链上行为：

* 维护 per-epoch randomness（可用 VRF 或 hash 链）
* 接收 storage node 广播的 micro-block
* verifier 随时可对 micro-block 做检查或仅做承诺更新
* committee 在 epoch 结束时聚合 micro-block 为 macro-block（模拟 “fused”）。

最小实现建议：

* 内存链：Vec<BlockEvent>
* micro-block 池：按 (file_id, epoch, round) 索引
* randomness：seed = H(prev_block_hash || epoch || round || domain)
* committee：固定比例抽样或 VRF 选择（先简化）

---

## 9. 测试策略（模块压测 + 模块间集成）

### 9.1 correctness 单元测试（必须覆盖）

conv：

1. Stage-1 cross-term 与承诺更新公式一致性测试

* 随机生成小参数 (N=8,n=8,k=2)，随机 F,R,G,H，构造 C(j)
* 按定义算 L_vec,R_vec，然后按公式更新 C(j+1/2)
* 同时显式算 Com(F(j+1/2);R(j+1/2))，比较是否相等
  依据推导展开等式在文稿里出现。

2. Stage-2 同理（odd-even 分解 + L_poly,R_poly + C 更新）

3. public_update_round 与 prover_round 的 C 更新结果一致

* prover 端跑一轮，拿到 micro-block
* verifier 端只做 public_update_round
* 比较双方 C(j+1) 一致

4. end-to-end：跑 R 轮后 final opening 检查通过（C(R)=μ·g*+ρ·h*）。

负例（anti-forgery 工程测试）：

* 篡改 micro-block 的 l_vec 或 r_vec 任意 1 bit，应导致后续挑战或最终检查失败
* 篡改 domain tag（把 vec 当 poly）应失败（确保 domain separation 生效）

twist：

* Lj,Rj 生成与 fold 一致
* final resolve 在 honest 情况通过，篡改 micro-block 失败

### 9.2 property tests（proptest）

* 随机小参数、多轮、多 epoch：检查不 panic，检查最终一致
* 随机插入网络乱序：chain 模块应能拒绝缺轮或错误 round 的 micro-block

### 9.3 fuzz（cargo-fuzz）

目标入口：

* micro-block 反序列化 + verifier update
* 避免 OOM、panic、整数溢出、指数级行为

---

## 10. 性能压测指标与 bench 设计（你要的数据在这里）

conv 的热点通常在：

* cross-term 计算：大量 ring*module 乘法与求和（可并行） 
* 多项式乘法后端（naive vs NTT）
* odd-even 分解与 fold（内存带宽）

建议 bench（criterion）：

模块级：

1. bench_conv_cross_terms_vec(N,n,k,backend,threads)
2. bench_conv_cross_terms_poly(N,n,k,backend,threads)
3. bench_conv_odd_even_decomp(n)
4. bench_conv_round_prover_total（含两阶段）
5. bench_conv_public_update_round（只更新 C）
6. bench_conv_final_opening_verify（重放 folds + 最终等式）

集成级（真实场景）：
7) bench_sim_epoch_throughput

* 固定文件数 Fcount、节点数 Scount、epoch 长度、网络延迟分布
* 统计：micro-block/s、最终确认延迟、verifier CPU 占用

输出建议：

* JSON/CSV：每个 bench 输出 time/op、alloc/op、peak RSS（可用 jemalloc 统计或 /proc/self/status）

---

## 11. CLI 任务清单（codex 直接照做）

damascus-cli 子命令建议：

* damascus-cli params gen --scheme conv --n0 2^R --k K --q 998244353 --seed <hex>
* damascus-cli file register --scheme conv --path data.bin --file-id auto

  * 输出 commitment C(0) 与初始 witness 存储位置
* damascus-cli run --scheme conv --epochs E --rounds R --nodes S --verifiers V --network-delay ms

  * 输出：每轮 prover 耗时、每轮 verifier update 耗时、最终 opening 验证耗时、失败率
* damascus-cli bench --preset small/medium/large
* damascus-cli corrupt --mode flip-bit --target mb-vec --round j

---

## 12. 实现顺序（降低返工）

1. damascus-types + damascus-crypto（hash 输入规范先固定）
2. damascus-ring（先 naive，确保 correctness；再加 NTT 后端）
3. damascus-conv（先 prover_round 与 public_update_round；再 final opening）
4. damascus-bench（先模块级 bench）
5. damascus-chain + damascus-sim（集成场景）
6. 最后补 twist（作为 baseline 与对照组）

---

## 13. 关键实现细节提醒（容易踩坑）

* 字节规范：challenge 派生的 input 必须包括当轮的 C(j) 或 C(j+1/2)，这在算法里是硬要求（否则可重放性会漂）。 
* domain separation：严格 "vec"/"poly"，并实现 counter 重试直到非 0。
* G/H 的折叠系数是 x^{-1}, y^{-1}，别写反（这是承诺不变量保持的关键）。 
* odd-even folding 是对每个 ring 元素做分解；不要把它误解成对向量 index 的奇偶分解（那会错）。
* conv 的 “常数时间更新” 指 verifier 每轮只做常数量 module 加法与标量乘法，不应触碰原始 F,R 数据。
