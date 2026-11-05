//! Phase 1.1 performance target: 1M vectors @ 768d, top-10 query < 50ms.
//!
//! Ignored by default (heavy: ~3GB RAM). Run explicitly:
//!   cargo test --release --test bench -- --ignored --nocapture

use std::time::{Duration, Instant};

use athenadb::core::{Entry, VectorStore};

/// Cheap deterministic xorshift fill — query cost is value-independent
/// (it's O(N·dim) dot products), so we don't need a real RNG for timing.
fn fill(v: &mut [f32], state: &mut u64) {
    for x in v.iter_mut() {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *x = ((*state >> 40) as f32 / (1u64 << 24) as f32) - 0.5;
    }
}

fn measure(n: usize, dim: usize) -> (Duration, Duration) {
    let mut store = VectorStore::new();
    let mut state = 0x9e37_79b9_7f4a_7c15u64;

    let t0 = Instant::now();
    for i in 0..n {
        let mut v = vec![0.0f32; dim];
        fill(&mut v, &mut state);
        store
            .upsert(Entry {
                id: format!("e{i}"),
                vector: v,
                metadata: serde_json::Value::Null,
                namespace: "default".into(),
                embed_model_id: "m".into(),
                inv_norm: 0.0,
            })
            .unwrap();
    }
    let build = t0.elapsed();

    let mut q = vec![0.0f32; dim];
    fill(&mut q, &mut state);

    let _ = store.search(&q, 10, None, None).unwrap(); // warm up

    // Median, not mean: one query preempted by another process shouldn't move the number.
    let mut times: Vec<Duration> = (0..31)
        .map(|_| {
            let t = Instant::now();
            let _ = store.search(&q, 10, None, None).unwrap();
            t.elapsed()
        })
        .collect();
    times.sort();
    (build, times[times.len() / 2])
}

#[test]
#[ignore]
fn bench_million_vectors() {
    let mut million_768 = Duration::ZERO;
    for dim in [384usize, 768] {
        for n in [10_000usize, 100_000, 1_000_000] {
            let (build, per) = measure(n, dim);
            println!("BENCH: {n} x {dim}d: built in {build:?}; median top-10 query = {per:?}");
            if n == 1_000_000 && dim == 768 {
                million_768 = per;
            }
        }
    }
    assert!(million_768.as_millis() < 50, "query exceeded 50ms target: {million_768:?}");
}
