#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadKind {
    Compute,
    Memory,
    Ipc,
    Io,
    FullSuite,
}

impl WorkloadKind {
    pub fn name(&self) -> &'static str {
        match self {
            WorkloadKind::Compute => "Compute Stress (Prime ALU & Cryptographic Hashing)",
            WorkloadKind::Memory => "Memory Subsystem (Sequential & Random Bandwidth/Latency)",
            WorkloadKind::Ipc => "IPC & Concurrency (Multi-Agent MPSC Channels & Contention)",
            WorkloadKind::Io => "Storage & Workspace I/O (Chunk Writes/Reads & Sync)",
            WorkloadKind::FullSuite => "Full System Multi-Subagent Suite",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    pub num_subagents: usize,
    pub duration_secs: u64,
    pub workload: WorkloadKind,
    pub verbose: bool,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        Self {
            num_subagents: cores,
            duration_secs: 3,
            workload: WorkloadKind::Compute,
            verbose: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SubagentMetrics {
    pub agent_id: usize,
    pub workload: String,
    pub ops_completed: u64,
    pub elapsed_nanos: u128,
    pub primary_metric_name: String,
    pub primary_metric_val: f64,
    pub secondary_metric_name: String,
    pub secondary_metric_val: f64,
    pub bytes_processed: u64,
}

impl SubagentMetrics {
    pub fn ops_per_sec(&self) -> f64 {
        if self.elapsed_nanos > 0 {
            (self.ops_completed as f64) / (self.elapsed_nanos as f64 / 1_000_000_000.0)
        } else {
            0.0
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum SubagentCommand {
    Start {
        workload: WorkloadKind,
        duration_secs: u64,
    },
    Stop,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum SubagentEvent {
    Ready { agent_id: usize },
    Started { agent_id: usize },
    Completed { agent_id: usize, metrics: SubagentMetrics },
    Failed { agent_id: usize, error: String },
}

#[derive(Debug, Clone)]
pub struct AggregatedBenchmarkReport {
    pub workload_name: String,
    pub num_subagents: usize,
    pub duration_secs: f64,
    pub total_ops: u64,
    pub aggregate_ops_per_sec: f64,
    pub per_agent_metrics: Vec<SubagentMetrics>,
    pub primary_summary: String,
    pub secondary_summary: String,
}
