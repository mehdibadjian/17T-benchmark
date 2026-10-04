use super::types::SubagentMetrics;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct IpcSubagent;

impl IpcSubagent {
    pub fn run(
        agent_id: usize,
        duration: Duration,
        stop_signal: Arc<AtomicBool>,
        shared_counter: Arc<AtomicU64>,
        shared_mutex: Arc<Mutex<Vec<u64>>>,
    ) -> SubagentMetrics {
        let start_time = Instant::now();
        let (tx, rx): (Sender<u64>, Receiver<u64>) = channel();

        let mut messages_sent: u64 = 0;
        let mut atomic_ops: u64 = 0;
        let mut mutex_ops: u64 = 0;

        let batch_size = 500;

        while !stop_signal.load(Ordering::Relaxed) && start_time.elapsed() < duration {
            // 1. Channel IPC messaging test: send and receive a batch of messages
            for i in 0..batch_size {
                let msg = (agent_id as u64) << 32 | i;
                let _ = tx.send(msg);
            }
            for _ in 0..batch_size {
                if rx.recv().is_ok() {
                    messages_sent += 1;
                }
            }

            // 2. Contended Atomic Operations (multi-agent shared counter)
            for _ in 0..batch_size {
                shared_counter.fetch_add(1, Ordering::Relaxed);
                atomic_ops += 1;
            }

            // 3. Mutex Contention (thread lock acquire, update, release)
            if let Ok(mut guard) = shared_mutex.try_lock() {
                guard.push(agent_id as u64);
                if guard.len() > 100 {
                    guard.clear();
                }
                mutex_ops += 1;
            }
        }

        let elapsed = start_time.elapsed();
        let elapsed_secs = elapsed.as_secs_f64().max(0.0001);
        let msgs_per_sec = (messages_sent as f64) / elapsed_secs;
        let atomics_per_sec = (atomic_ops as f64) / elapsed_secs;
        let total_ops = messages_sent + atomic_ops + mutex_ops;

        SubagentMetrics {
            agent_id,
            workload: "IPC & Concurrency".to_string(),
            ops_completed: total_ops,
            elapsed_nanos: elapsed.as_nanos(),
            primary_metric_name: "Channel Msg/s".to_string(),
            primary_metric_val: msgs_per_sec,
            secondary_metric_name: "Contended Atomics/s".to_string(),
            secondary_metric_val: atomics_per_sec,
            bytes_processed: messages_sent * 8,
        }
    }
}
