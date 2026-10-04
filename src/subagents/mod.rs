pub mod compute;
pub mod io;
pub mod ipc;
pub mod memory;
pub mod types;

use compute::ComputeSubagent;
use io::IoSubagent;
use ipc::IpcSubagent;
use memory::MemorySubagent;
pub use types::*;

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub struct SubagentPool;

impl SubagentPool {
    pub fn run_benchmark(config: &BenchmarkConfig) -> AggregatedBenchmarkReport {
        let num_agents = config.num_subagents.max(1);
        let duration = Duration::from_secs(config.duration_secs);
        let start_barrier = Arc::new(Barrier::new(num_agents + 1));
        let stop_signal = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel::<SubagentEvent>();

        // Shared resources for IPC workload
        let shared_counter = Arc::new(AtomicU64::new(0));
        let shared_mutex = Arc::new(Mutex::new(Vec::with_capacity(1000)));

        if config.verbose {
            println!(
                "\x1b[1;34m[Subagent Pool]\x1b[0m Spawning \x1b[1;32m{} subagents\x1b[0m for workload: \x1b[1m{}\x1b[0m (duration: {}s)...",
                num_agents,
                config.workload.name(),
                config.duration_secs
            );
        }

        let mut handles = Vec::with_capacity(num_agents);

        for agent_id in 0..num_agents {
            let barrier = Arc::clone(&start_barrier);
            let stop = Arc::clone(&stop_signal);
            let tx_clone: Sender<SubagentEvent> = tx.clone();
            let workload = config.workload;
            let counter = Arc::clone(&shared_counter);
            let mutex = Arc::clone(&shared_mutex);

            let handle = thread::Builder::new()
                .name(format!("subagent-{}", agent_id))
                .spawn(move || {
                    let _ = tx_clone.send(SubagentEvent::Ready { agent_id });
                    // Wait for all subagents to synchronize
                    barrier.wait();
                    let _ = tx_clone.send(SubagentEvent::Started { agent_id });

                    let metrics = match workload {
                        WorkloadKind::Compute => ComputeSubagent::run(agent_id, duration, stop),
                        WorkloadKind::Memory => MemorySubagent::run(agent_id, duration, stop),
                        WorkloadKind::Ipc => IpcSubagent::run(agent_id, duration, stop, counter, mutex),
                        WorkloadKind::Io => IoSubagent::run(agent_id, duration, stop),
                        WorkloadKind::FullSuite => {
                            // Cycle through different sub-tasks for a composite load
                            ComputeSubagent::run(agent_id, duration, stop)
                        }
                    };

                    let _ = tx_clone.send(SubagentEvent::Completed { agent_id, metrics });
                });

            if let Ok(h) = handle {
                handles.push(h);
            }
        }

        // Parent orchestrator waits for all subagents to initialize
        let mut ready_count = 0;
        while ready_count < num_agents {
            if let Ok(SubagentEvent::Ready { .. }) = rx.recv() {
                ready_count += 1;
            }
        }

        // Release the barrier: all subagents start simultaneously!
        let bench_start = Instant::now();
        start_barrier.wait();

        // Collect results
        let mut results = Vec::with_capacity(num_agents);
        let mut _started_count = 0;

        while results.len() < num_agents {
            match rx.recv() {
                Ok(SubagentEvent::Started { .. }) => {
                    _started_count += 1;
                }
                Ok(SubagentEvent::Completed { metrics, .. }) => {
                    results.push(metrics);
                }
                Ok(SubagentEvent::Failed { agent_id, error }) => {
                    eprintln!("\x1b[31mSubagent {} failed: {}\x1b[0m", agent_id, error);
                    results.push(SubagentMetrics {
                        agent_id,
                        workload: format!("Failed: {}", error),
                        ..Default::default()
                    });
                }
                _ => {}
            }
        }

        for h in handles {
            let _ = h.join();
        }

        let total_wall_time = bench_start.elapsed().as_secs_f64().max(0.0001);

        // Sort results by agent ID
        results.sort_by_key(|m| m.agent_id);

        let total_ops: u64 = results.iter().map(|m| m.ops_completed).sum();
        let aggregate_ops_per_sec = (total_ops as f64) / total_wall_time;

        let primary_sum: f64 = results.iter().map(|m| m.primary_metric_val).sum();
        let secondary_sum: f64 = results.iter().map(|m| m.secondary_metric_val).sum();

        let primary_metric_name = results
            .first()
            .map(|m| m.primary_metric_name.as_str())
            .unwrap_or("Score");

        let primary_summary = format!("{:.2} (Aggregate {})", primary_sum, primary_metric_name);
        let secondary_summary = format!("{:.2} (Combined)", secondary_sum);

        let report = AggregatedBenchmarkReport {
            workload_name: config.workload.name().to_string(),
            num_subagents: num_agents,
            duration_secs: total_wall_time,
            total_ops,
            aggregate_ops_per_sec,
            per_agent_metrics: results,
            primary_summary,
            secondary_summary,
        };

        if config.verbose {
            Self::display_report(&report);
        }

        report
    }

    pub fn display_report(report: &AggregatedBenchmarkReport) {
        println!();
        println!("\x1b[1;36m┌────────────────────────────────────────────────────────────────────────┐\x1b[0m");
        println!(
            "\x1b[1;36m│\x1b[0m \x1b[1;32mPERFORMANCE BENCHMARK REPORT: {:<40}\x1b[0m \x1b[1;36m│\x1b[0m",
            report.workload_name
        );
        println!("\x1b[1;36m├────────────────────────────────────────────────────────────────────────┤\x1b[0m");
        println!(
            "│ \x1b[1mActive Subagents\x1b[0m:  {:<10}  \x1b[1mTest Duration\x1b[0m: {:<6.2} s            │",
            report.num_subagents, report.duration_secs
        );
        println!(
            "│ \x1b[1mTotal Operations\x1b[0m:  {:<10}  \x1b[1mAggregate Rate\x1b[0m: {:<10.0} ops/s  │",
            report.total_ops, report.aggregate_ops_per_sec
        );
        println!(
            "│ \x1b[1mPrimary Metric\x1b[0m:    {:<51} │",
            report.primary_summary
        );
        println!("\x1b[1;36m├──────────┬─────────────────┬─────────────────┬─────────────────────────┤\x1b[0m");
        println!("\x1b[1;33m│ Agent ID │ Operations      │ Ops/Sec         │ Primary Metric          │\x1b[0m");
        println!("\x1b[1;36m├──────────┼─────────────────┼─────────────────┼─────────────────────────┤\x1b[0m");

        for m in &report.per_agent_metrics {
            println!(
                "│ Agent #{:<2} │ {:<15} │ {:<15.0} │ {:<10.2} {:<12}│",
                m.agent_id,
                m.ops_completed,
                m.ops_per_sec(),
                m.primary_metric_val,
                m.primary_metric_name
            );
        }

        println!("\x1b[1;36m└──────────┴─────────────────┴─────────────────┴─────────────────────────┘\x1b[0m");
        println!();
    }

    pub fn run_scaling_analysis(
        workload: WorkloadKind,
        max_agents: usize,
        duration_per_test: u64,
    ) {
        let agent_counts: Vec<usize> = [1, 2, 4, 8, 16]
            .into_iter()
            .filter(|&c| c <= max_agents)
            .collect();

        println!("\x1b[1;35m========================================================================\x1b[0m");
        println!(
            "\x1b[1;32m      MULTI-SUBAGENT SCALING & EFFICIENCY CHECK: {}\x1b[0m",
            workload.name()
        );
        println!("\x1b[1;35m========================================================================\x1b[0m");

        let mut baseline_ops_per_sec = 0.0;
        let mut reports = Vec::new();

        for &count in &agent_counts {
            let config = BenchmarkConfig {
                num_subagents: count,
                duration_secs: duration_per_test,
                workload,
                verbose: false,
            };

            print!("  Testing with {:>2} subagent(s)... ", count);
            std::io::Write::flush(&mut std::io::stdout()).unwrap();

            let report = Self::run_benchmark(&config);
            if count == 1 {
                baseline_ops_per_sec = report.aggregate_ops_per_sec;
            }
            println!(
                "\x1b[32m{:>10.0} ops/s\x1b[0m (Total: {})",
                report.aggregate_ops_per_sec, report.total_ops
            );
            reports.push((count, report));
        }

        println!("\x1b[1;35m------------------------------------------------------------------------\x1b[0m");
        println!("\x1b[1;33m Agents │ Aggregate Ops/s │ Speedup vs 1-Agent │ Multi-Core Efficiency   \x1b[0m");
        println!("\x1b[1;35m------------------------------------------------------------------------\x1b[0m");

        for (count, report) in &reports {
            let speedup = if baseline_ops_per_sec > 0.0 {
                report.aggregate_ops_per_sec / baseline_ops_per_sec
            } else {
                1.0
            };
            let efficiency = (speedup / (*count as f64)) * 100.0;

            let eff_color = if efficiency >= 85.0 {
                "\x1b[32m" // Green
            } else if efficiency >= 60.0 {
                "\x1b[33m" // Yellow
            } else {
                "\x1b[31m" // Red
            };

            println!(
                "   {:>2}   │ {:>13.0}   │ {:>16.2}x   │ {}{:>19.1}%  \x1b[0m",
                count, report.aggregate_ops_per_sec, speedup, eff_color, efficiency
            );
        }
        println!("\x1b[1;35m========================================================================\x1b[0m");
        println!();
    }
}
