use super::types::SubagentMetrics;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct ComputeSubagent;

impl ComputeSubagent {
    pub fn run(
        agent_id: usize,
        duration: Duration,
        stop_signal: Arc<AtomicBool>,
    ) -> SubagentMetrics {
        let start_time = Instant::now();
        let mut total_ops: u64 = 0;
        let mut prime_checks: u64 = 0;
        let mut hash_blocks: u64 = 0;

        let mut candidate: u64 = 1_000_000_000 + (agent_id as u64 * 50_000);
        let mut hash_state = [
            0x6a09e667u32, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
            0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
        ];
        let block = [0x5au8; 64];

        while !stop_signal.load(Ordering::Relaxed) && start_time.elapsed() < duration {
            // 1. Prime checking loop (50 iterations per batch)
            for _ in 0..50 {
                if is_prime(candidate) {
                    prime_checks += 1;
                }
                candidate += 2;
            }

            // 2. Cryptographic SHA-256 compression rounds (20 blocks per batch)
            for _ in 0..20 {
                hash_state = sha256_compress_block(hash_state, &block);
                black_box(&hash_state);
                hash_blocks += 1;
            }

            // 3. Matrix dot product (16x16 float multiplication)
            black_box(matrix_dot_16());

            total_ops += 71; // 50 primes + 20 hashes + 1 matrix
        }

        let elapsed = start_time.elapsed();
        let elapsed_secs = elapsed.as_secs_f64().max(0.0001);
        let mhashes_per_sec = (hash_blocks as f64 / 1_000_000.0) / elapsed_secs;
        let prime_ops_per_sec = (prime_checks as f64) / elapsed_secs;

        SubagentMetrics {
            agent_id,
            workload: "Compute".to_string(),
            ops_completed: total_ops,
            elapsed_nanos: elapsed.as_nanos(),
            primary_metric_name: "Hash Throughput".to_string(),
            primary_metric_val: mhashes_per_sec,
            secondary_metric_name: "Prime Checks/s".to_string(),
            secondary_metric_val: prime_ops_per_sec,
            bytes_processed: hash_blocks * 64,
        }
    }
}

#[inline(always)]
fn is_prime(n: u64) -> bool {
    if n <= 1 {
        return false;
    }
    if n <= 3 {
        return true;
    }
    if n % 2 == 0 || n % 3 == 0 {
        return false;
    }
    let mut i = 5;
    while i * i <= n {
        if n % i == 0 || n % (i + 2) == 0 {
            return false;
        }
        i += 6;
    }
    true
}

#[inline(always)]
fn sha256_compress_block(state: [u32; 8], block: &[u8; 64]) -> [u32; 8] {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;

    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];

    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ (!e & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    [
        state[0].wrapping_add(a),
        state[1].wrapping_add(b),
        state[2].wrapping_add(c),
        state[3].wrapping_add(d),
        state[4].wrapping_add(e),
        state[5].wrapping_add(f),
        state[6].wrapping_add(g),
        state[7].wrapping_add(h),
    ]
}

#[inline(always)]
fn matrix_dot_16() -> f32 {
    let mut sum = 0.0f32;
    let a = [1.05f32; 16];
    let b = [0.95f32; 16];
    for i in 0..16 {
        sum += a[i] * b[i];
    }
    sum
}
