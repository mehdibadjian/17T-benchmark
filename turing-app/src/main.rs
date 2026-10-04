use std::env;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use turing_app::actor::{ActorSystem, MetricsActor, StorageActor};
use turing_app::cli::CliClient;
use turing_app::server::{HttpServer, ServerConfig};
use turing_app::storage::LsmStorageEngine;

const DEFAULT_PORT: u16 = 8088;
const DEFAULT_DATA_DIR: &str = "/workspace/nimble-turing/.turing_data";
const DEFAULT_BENCH_OPS: usize = 20_000;
const DEFAULT_BENCH_THREADS: usize = 8;

struct CliArgs {
    port: u16,
    data_dir: PathBuf,
    bench: Option<(usize, usize)>,
    interactive: bool,
    help: bool,
}

fn parse_args() -> Result<CliArgs, String> {
    let raw_args: Vec<String> = env::args().skip(1).collect();
    let mut port = DEFAULT_PORT;
    let mut data_dir = PathBuf::from(DEFAULT_DATA_DIR);
    let mut bench = None;
    let mut interactive = false;
    let mut help = false;

    let mut i = 0;
    while i < raw_args.len() {
        let arg = &raw_args[i];
        match arg.as_str() {
            "-h" | "--help" => {
                help = true;
                i += 1;
            }
            "-p" | "--port" => {
                if i + 1 < raw_args.len() {
                    port = raw_args[i + 1]
                        .parse::<u16>()
                        .map_err(|e| format!("Invalid port '{}': {}", raw_args[i + 1], e))?;
                    i += 2;
                } else {
                    return Err("Missing value for --port".to_string());
                }
            }
            "-d" | "--data-dir" => {
                if i + 1 < raw_args.len() {
                    data_dir = PathBuf::from(&raw_args[i + 1]);
                    i += 2;
                } else {
                    return Err("Missing value for --data-dir".to_string());
                }
            }
            "--bench" => {
                let mut ops = DEFAULT_BENCH_OPS;
                let mut threads = DEFAULT_BENCH_THREADS;
                i += 1;

                if i < raw_args.len() && !raw_args[i].starts_with('-') {
                    if let Ok(val) = raw_args[i].parse::<usize>() {
                        ops = val;
                        i += 1;
                        if i < raw_args.len() && !raw_args[i].starts_with('-') {
                            if let Ok(t_val) = raw_args[i].parse::<usize>() {
                                threads = t_val;
                                i += 1;
                            }
                        }
                    }
                }
                bench = Some((ops, threads));
            }
            "-i" | "--interactive" | "--cli" => {
                interactive = true;
                i += 1;
            }
            unknown => {
                return Err(format!("Unknown option '{}'. Use --help for usage.", unknown));
            }
        }
    }

    Ok(CliArgs {
        port,
        data_dir,
        bench,
        interactive,
        help,
    })
}

fn print_help() {
    println!(
        r#"Turing Storage Engine & Server v0.1.0
High-performance LSM storage engine and concurrent Actor system in Rust

USAGE:
    turing-server [OPTIONS]

OPTIONS:
    --port <PORT>               Port to listen on (default: 8088)
    --data-dir <DIR>            LSM storage data directory (default: /workspace/nimble-turing/.turing_data)
    --bench [OPS] [THREADS]     Run multi-threaded stress benchmark (default: 20000 ops, 8 threads)
    --interactive, --cli        Launch interactive REPL client
    -h, --help                  Print this help information

EXAMPLES:
    turing-server                               Start server with default settings
    turing-server --port 9090                   Start server on port 9090
    turing-server --bench 50000 8               Run 50k ops benchmark with 8 threads
    turing-server --interactive                 Launch interactive REPL client
"#
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = match parse_args() {
        Ok(a) => a,
        Err(err) => {
            eprintln!("Error: {}", err);
            eprintln!("Try 'turing-server --help' for more information.");
            std::process::exit(1);
        }
    };

    if args.help {
        print_help();
        return Ok(());
    }

    if let Some((ops, threads)) = args.bench {
        println!("Initializing Turing benchmark suite (ops={}, threads={})...", ops, threads);
        // Check if server is running on target port
        let server_addr = format!("127.0.0.1:{}", args.port);
        let mut client = if TcpStream::connect(&server_addr).is_ok() {
            println!("Connecting to running server at {}...", server_addr);
            CliClient::new_remote(server_addr)
        } else {
            println!("No running server on {}. Benchmarking local LSM storage engine...", server_addr);
            std::fs::create_dir_all(&args.data_dir)?;
            let engine = LsmStorageEngine::open(&args.data_dir)?;
            CliClient::new_local(engine)
        };

        let report = client.run_benchmark(ops, threads);
        println!("{}", report);
        return Ok(());
    }

    if args.interactive {
        let server_addr = format!("127.0.0.1:{}", args.port);
        let mut client = if TcpStream::connect(&server_addr).is_ok() {
            println!("Connected to server at {}", server_addr);
            CliClient::new_remote(server_addr)
        } else {
            println!("No active server on {}. Operating on local LSM engine at {:?}...", server_addr, args.data_dir);
            std::fs::create_dir_all(&args.data_dir)?;
            let engine = LsmStorageEngine::open(&args.data_dir)?;
            CliClient::new_local(engine)
        };

        if let Err(err) = client.run_interactive() {
            eprintln!("REPL error: {}", err);
        }
        return Ok(());
    }

    // Default: Start Server
    std::fs::create_dir_all(&args.data_dir)?;
    let engine = Arc::new(RwLock::new(LsmStorageEngine::open(&args.data_dir)?));

    let system = ActorSystem::new("turing-system");
    let storage_actor = system
        .spawn("storage", StorageActor::with_engine(Arc::clone(&engine)))
        .map_err(|e| format!("Failed to spawn StorageActor: {}", e))?;
    let metrics_actor = system
        .spawn("metrics", MetricsActor::new())
        .map_err(|e| format!("Failed to spawn MetricsActor: {}", e))?;

    let config = ServerConfig {
        addr: format!("0.0.0.0:{}", args.port),
        worker_threads: 8,
        read_timeout: Duration::from_secs(30),
        write_timeout: Duration::from_secs(30),
    };

    let server = HttpServer::with_actors(config, system.clone(), storage_actor, metrics_actor);
    let _handle = server.bind().map_err(|e| format!("Failed to bind server: {}", e))?;

    println!("======================================================================");
    println!("  _____ _   _ ____  ___ _   _  ____   ____  _____ ______     _______ ____  ");
    println!(" |_   _| | | |  _ \\|_ _| \\ | |/ ___| / ___|| ____|  _ \\ \\   / / ____|  _ \\ ");
    println!("   | | | | | | |_) || ||  \\| | |  _  \\___ \\|  _| | |_) \\ \\ / /|  _| | |_) |");
    println!("   | | | |_| |  _ < | || |\\  | |_| |  ___) | |___|  _ < \\ V / | |___|  _ < ");
    println!("   |_|  \\___/|_| \\_\\___|_| \\_|\\____| |____/|_____|_| \\_\\ \\_/  |_____|_| \\_\\");
    println!("======================================================================");
    println!("  LSM Storage Path    : {}", args.data_dir.display());
    println!("  Server Listening    : http://0.0.0.0:{}", args.port);
    println!("  Line Protocol       : 0.0.0.0:{} (commands: PUT, GET, DEL, SCAN, STATS, PING)", args.port);
    println!("  HTTP REST API       : GET /health, GET /api/v1/get?key=..., POST /api/v1/put,");
    println!("                        DELETE /api/v1/delete?key=..., GET /api/v1/scan, GET /api/v1/stats");
    println!("  Worker Threads      : 8");
    println!("======================================================================");
    println!("Server is running. Press Ctrl+C to terminate.");

    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
