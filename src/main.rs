mod shell;
mod subagents;
mod sysinfo;

use shell::Shell;
use std::env;
use std::path::Path;

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut shell = Shell::new();

    if args.len() > 1 {
        match args[1].as_str() {
            "-c" => {
                if args.len() > 2 {
                    let cmd_line = args[2..].join(" ");
                    let code = shell.execute_string(&cmd_line);
                    std::process::exit(code);
                } else {
                    eprintln!("nimble-shell: -c requires a command string");
                    std::process::exit(1);
                }
            }
            "bench" => {
                let bench_cmd = args[1..].join(" ");
                let code = shell.execute_string(&bench_cmd);
                std::process::exit(code);
            }
            "sysinfo" | "device" => {
                let code = shell.execute_string(&args[1]);
                std::process::exit(code);
            }
            "--help" | "-h" => {
                println!("\x1b[1mUsage:\x1b[0m nimble-shell [OPTIONS] [SCRIPT]");
                println!();
                println!("Options:");
                println!("  -c <COMMAND>    Execute command string non-interactively");
                println!("  bench [ARGS]    Run multi-subagent performance check directly");
                println!("  sysinfo         Inspect device hardware and operating system");
                println!("  -h, --help      Show this help message");
                println!();
                println!("When no arguments are provided, nimble-shell starts an interactive REPL.");
                std::process::exit(0);
            }
            script_path => {
                let path = Path::new(script_path);
                if path.exists() {
                    let code = shell.execute_file(path);
                    std::process::exit(code);
                } else {
                    eprintln!("nimble-shell: no such file: {}", script_path);
                    std::process::exit(1);
                }
            }
        }
    } else {
        // Interactive REPL
        shell.run_repl();
    }
}
