use super::types::SubagentMetrics;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct IoSubagent;

impl IoSubagent {
    pub fn run(
        agent_id: usize,
        duration: Duration,
        stop_signal: Arc<AtomicBool>,
    ) -> SubagentMetrics {
        let start_time = Instant::now();
        let file_path = PathBuf::from(format!("/workspace/nimble-turing/.bench_io_{}.tmp", agent_id));

        // 64 KB block size
        let block_size = 64 * 1024;
        let write_buf = vec![0x3cu8 ^ (agent_id as u8); block_size];
        let mut read_buf = vec![0u8; block_size];

        let mut bytes_written: u64 = 0;
        let mut bytes_read: u64 = 0;
        let mut iops: u64 = 0;

        let file_res = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&file_path);

        if let Ok(mut file) = file_res {
            let max_file_blocks = 256; // Up to 16MB per agent file

            while !stop_signal.load(Ordering::Relaxed) && start_time.elapsed() < duration {
                // Rewind and write blocks
                let _ = file.seek(SeekFrom::Start(0));
                for _ in 0..max_file_blocks {
                    if stop_signal.load(Ordering::Relaxed) || start_time.elapsed() >= duration {
                        break;
                    }
                    if file.write_all(&write_buf).is_ok() {
                        bytes_written += block_size as u64;
                        iops += 1;
                    }
                }
                let _ = file.flush();

                // Rewind and read blocks
                let _ = file.seek(SeekFrom::Start(0));
                for _ in 0..max_file_blocks {
                    if stop_signal.load(Ordering::Relaxed) || start_time.elapsed() >= duration {
                        break;
                    }
                    if let Ok(n) = file.read(&mut read_buf) {
                        if n == 0 {
                            break;
                        }
                        bytes_read += n as u64;
                        iops += 1;
                    }
                }
            }
        }

        // Clean up temporary benchmark file
        let _ = fs::remove_file(&file_path);

        let elapsed = start_time.elapsed();
        let elapsed_secs = elapsed.as_secs_f64().max(0.0001);
        let write_mb_s = (bytes_written as f64 / 1_048_576.0) / elapsed_secs;
        let read_mb_s = (bytes_read as f64 / 1_048_576.0) / elapsed_secs;
        let combined_mb_s = ((bytes_written + bytes_read) as f64 / 1_048_576.0) / elapsed_secs;

        SubagentMetrics {
            agent_id,
            workload: "Storage I/O".to_string(),
            ops_completed: iops,
            elapsed_nanos: elapsed.as_nanos(),
            primary_metric_name: "Combined I/O MB/s".to_string(),
            primary_metric_val: combined_mb_s,
            secondary_metric_name: format!("W:{:.1} R:{:.1} MB/s", write_mb_s, read_mb_s),
            secondary_metric_val: write_mb_s,
            bytes_processed: bytes_written + bytes_read,
        }
    }
}
