use super::types::SubagentMetrics;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct MemorySubagent;

impl MemorySubagent {
    pub fn run(
        agent_id: usize,
        duration: Duration,
        stop_signal: Arc<AtomicBool>,
    ) -> SubagentMetrics {
        let start_time = Instant::now();
        // 16 MB buffer per subagent
        let buffer_size = 16 * 1024 * 1024;
        let mut buffer: Vec<u8> = vec![0u8; buffer_size];

        let mut bytes_written: u64 = 0;
        let mut bytes_read: u64 = 0;
        let mut total_ops: u64 = 0;

        let pattern = (0xaa ^ (agent_id as u8)) as u8;

        // Precompute random jump indices for latency test (256K entries)
        let lcg_count = 262_144;
        let mut random_indices: Vec<usize> = Vec::with_capacity(lcg_count);
        let mut lcg_state: u64 = 123456789 + (agent_id as u64 * 98765);
        for _ in 0..lcg_count {
            lcg_state = lcg_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let idx = ((lcg_state >> 16) as usize) % (buffer_size - 8);
            random_indices.push(idx);
        }

        while !stop_signal.load(Ordering::Relaxed) && start_time.elapsed() < duration {
            // 1. Sequential Write (fills the 16MB buffer)
            for chunk in buffer.chunks_exact_mut(8) {
                chunk.copy_from_slice(&[pattern; 8]);
            }
            bytes_written += buffer_size as u64;

            // 2. Sequential Read with accumulator checksum
            let mut sum: u64 = 0;
            for chunk in buffer.chunks_exact(8) {
                let val = u64::from_ne_bytes(chunk.try_into().unwrap());
                sum = sum.wrapping_add(val);
            }
            black_box(sum);
            bytes_read += buffer_size as u64;

            // 3. Random Memory Latency Test (pointer chasing across the buffer)
            let mut acc: u8 = 0;
            for &idx in &random_indices {
                acc = acc.wrapping_add(buffer[idx]);
            }
            black_box(acc);
            total_ops += (buffer_size / 8 * 2 + lcg_count) as u64;
        }

        let elapsed = start_time.elapsed();
        let elapsed_secs = elapsed.as_secs_f64().max(0.0001);

        let total_gb = (bytes_read + bytes_written) as f64 / 1_073_741_824.0;
        let throughput_gb_s = total_gb / elapsed_secs;
        let read_gb_s = (bytes_read as f64 / 1_073_741_824.0) / elapsed_secs;
        let write_gb_s = (bytes_written as f64 / 1_073_741_824.0) / elapsed_secs;

        SubagentMetrics {
            agent_id,
            workload: "Memory".to_string(),
            ops_completed: total_ops,
            elapsed_nanos: elapsed.as_nanos(),
            primary_metric_name: "Combined Bandwidth".to_string(),
            primary_metric_val: throughput_gb_s,
            secondary_metric_name: format!("R:{:.1} W:{:.1} GB/s", read_gb_s, write_gb_s),
            secondary_metric_val: read_gb_s,
            bytes_processed: bytes_read + bytes_written,
        }
    }
}
