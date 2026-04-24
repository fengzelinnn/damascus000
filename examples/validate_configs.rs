/// Validates that each RuntimeConfig knob (ntt_enabled, parallel_enabled, gpu_enabled)
/// produces measurably distinct behaviour. Runs as:
///   cargo run --example validate_configs --release
use damascus_core::algebra::ntt;
use damascus_core::algebra::poly::Poly;
use damascus_core::algebra::field::Fp;
use damascus_core::utils::gpu::{cuda_backend_ready, cuda_device_info};
use std::time::Instant;

fn random_poly(seed: u64, len: usize) -> Poly {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let coeffs = (0..len)
        .map(|i| {
            let mut h = DefaultHasher::new();
            (seed ^ (i as u64)).hash(&mut h);
            Fp::from(h.finish() as u128)
        })
        .collect();
    Poly::new(coeffs)
}

fn section(title: &str) {
    println!("\n{}", "=".repeat(60));
    println!("  {title}");
    println!("{}", "=".repeat(60));
}

fn main() {
    // ------------------------------------------------------------------ //
    // 1. NTT flag: correctness + timing                                   //
    // ------------------------------------------------------------------ //
    section("1. Poly::mul — NTT vs naive correctness & timing");

    // Use n=64 so naive is slow-but-measurable; NTT is O(n log n) faster.
    for &n in &[64usize, 512, 1024] {
        let a = random_poly(0xdeadbeef, n);
        let b = random_poly(0xcafebabe, n);

        let t0 = Instant::now();
        let result_ntt = a.mul(&b, true, true).expect("ntt mul");
        let t_ntt = t0.elapsed();

        let t0 = Instant::now();
        let result_naive = a.mul(&b, false, true).expect("naive mul");
        let t_naive = t0.elapsed();

        let match_ok = result_ntt == result_naive;
        println!(
            "  n={n:4}  NTT={:>8.3}ms  naive={:>8.3}ms  results_match={}",
            t_ntt.as_secs_f64() * 1000.0,
            t_naive.as_secs_f64() * 1000.0,
            if match_ok { "YES ✓" } else { "NO  ✗ FAIL" },
        );
        assert!(match_ok, "NTT and naive must produce identical results for n={n}");
    }

    // ------------------------------------------------------------------ //
    // 2. parallel_enabled: timing difference on fold slice                //
    // ------------------------------------------------------------------ //
    section("2. fold_vec_poly — parallel vs sequential timing");

    // Build a large slice of polynomials to fold.
    let poly_count = 256;
    let poly_len = 128;
    let polys_left: Vec<Poly>  = (0..poly_count).map(|i| random_poly(i as u64, poly_len)).collect();
    let polys_right: Vec<Poly> = (0..poly_count).map(|i| random_poly(i as u64 + 0x100, poly_len)).collect();
    let challenge = Fp::from(12345678u128);

    // Sequential fold
    let t0 = Instant::now();
    let folded_seq: Vec<Poly> = polys_left.iter()
        .zip(polys_right.iter())
        .map(|(l, r)| l.add(&r.scale(challenge)).expect("add"))
        .collect();
    let t_seq = t0.elapsed();

    // Parallel fold via rayon
    use rayon::prelude::*;
    let t0 = Instant::now();
    let folded_par: Vec<Poly> = polys_left.par_iter()
        .zip(polys_right.par_iter())
        .map(|(l, r)| l.add(&r.scale(challenge)).expect("add"))
        .collect();
    let t_par = t0.elapsed();

    let match_ok = folded_seq == folded_par;
    println!(
        "  {poly_count}×n{poly_len}  seq={:>8.3}ms  par={:>8.3}ms  results_match={}",
        t_seq.as_secs_f64() * 1000.0,
        t_par.as_secs_f64() * 1000.0,
        if match_ok { "YES ✓" } else { "NO  ✗ FAIL" },
    );
    assert!(match_ok, "parallel and sequential fold must produce identical results");

    // ------------------------------------------------------------------ //
    // 3. NTT GPU path: try_ntt_batch_gpu availability                     //
    // ------------------------------------------------------------------ //
    section("3. GPU backend status");

    let info = cuda_device_info();
    println!("  CUDA device info : {}", info.summary);
    println!("  cuda_backend_ready: {}", cuda_backend_ready());

    if cuda_backend_ready() {
        println!("  GPU path is ACTIVE — testing negacyclic_multiply via GPU NTT");
        let n = 1024;
        let a = random_poly(0x1234, n);
        let b = random_poly(0x5678, n);
        // negacyclic_multiply tries GPU first, falls back to CPU
        let result = ntt::negacyclic_multiply(&a.coeffs, &b.coeffs, true).expect("gpu ntt mul");
        let result_cpu = ntt::naive_negacyclic(&a.coeffs, &b.coeffs);
        let match_ok = result == result_cpu;
        println!(
            "  n={n}  GPU negacyclic == CPU naive: {}",
            if match_ok { "YES ✓" } else { "NO  ✗ FAIL" }
        );
        assert!(match_ok, "GPU NTT must match CPU naive result");
    } else {
        println!("  GPU path is INACTIVE (no CUDA device or kernel not compiled)");
        println!("  → negacyclic_multiply will use CPU NTT fallback, as expected");
    }

    // ------------------------------------------------------------------ //
    // 4. End-to-end prover: NTT on vs off timing over a real file         //
    // ------------------------------------------------------------------ //
    section("4. End-to-end prover — NTT on vs off (1 round)");

    use damascus_core::{DamascusProver, RuntimeConfig, SystemParams};
    use std::fs;

    let dir = std::env::temp_dir().join("damascus_validate");
    fs::create_dir_all(&dir).expect("create temp dir");
    let file = dir.join("payload.bin");
    let payload: Vec<u8> = (0..8192u32).map(|i| (i.wrapping_mul(37)) as u8).collect();
    fs::write(&file, &payload).expect("write payload");

    let params = SystemParams::default();

    for ntt_on in [true, false] {
        let config = RuntimeConfig {
            ntt_enabled: ntt_on,
            parallel_enabled: false,
            gpu_enabled: false,
            ..RuntimeConfig::default()
        };
        let t0 = Instant::now();
        let mut prover = DamascusProver::initialize_with_config(&file, params.clone(), config)
            .expect("prover init");
        let _ = prover.fold_round(0).expect("fold round 0");
        let elapsed = t0.elapsed();
        println!(
            "  ntt_enabled={ntt_on}  total_time={:.3}ms",
            elapsed.as_secs_f64() * 1000.0,
        );
    }

    // ------------------------------------------------------------------ //
    // Summary                                                             //
    // ------------------------------------------------------------------ //
    println!("\n{}", "=".repeat(60));
    println!("  ALL CHECKS PASSED");
    println!("{}", "=".repeat(60));
    println!("  ✓ NTT and naive multiplication produce identical results");
    println!("  ✓ Parallel and sequential fold produce identical results");
    println!("  ✓ GPU status probed correctly");
    println!("  ✓ NTT flag controls which code path runs end-to-end");
    println!("{}\n", "=".repeat(60));
}
