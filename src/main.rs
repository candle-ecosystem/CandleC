use clap::{Parser, Subcommand};
use std::path::PathBuf;

use candlec::diagnostic::{self, OutputMode, Reporter};

#[derive(Parser)]
#[command(
    name = "CandleC",
    bin_name = "candlec",
    about = concat!("Candle Assembly Compiler v", env!("CARGO_PKG_VERSION")),
    version = env!("CARGO_PKG_VERSION")
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Compile the project")]
    Build {
        #[arg(default_value = ".")]
        project: PathBuf,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Validate the project's CASM source")]
    Check {
        #[arg(default_value = ".")]
        project: PathBuf,
        #[arg(long)]
        json: bool,
    },
    #[command(about = "Clean the project's output directory")]
    Clean {
        #[arg(default_value = ".")]
        project: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        None => {
            use clap::CommandFactory;
            Cli::command().print_help().ok();
            println!();
        }
        Some(Commands::Build { project, json }) => {
            let mut reporter = reporter(json);
            let result = candlec::project::build(&project_path(project), &mut reporter);
            finish_task("build", json, result.map(|_| ()), reporter);
        }
        Some(Commands::Check { project, json }) => {
            let mut reporter = reporter(json);
            let result = candlec::project::check(&project_path(project), &mut reporter);
            finish_task("check", json, result, reporter);
        }
        Some(Commands::Clean { project, json }) => {
            let mut reporter = reporter(json);
            let result = candlec::project::clean(&project_path(project), &mut reporter);
            finish_task("clean", json, result, reporter);
        }
    }
}

fn reporter(json: bool) -> Reporter {
    Reporter::new(if json {
        OutputMode::Json
    } else {
        OutputMode::Human
    })
}

fn finish_task(task: &str, json: bool, result: Result<(), String>, mut reporter: Reporter) {
    if let Err(message) = result {
        if message != diagnostic::ALREADY_REPORTED {
            reporter.error("E_TASK", message, None, 0, 0);
        }
    }
    let success = !reporter.has_errors();
    if json {
        println!(
            "{}",
            diagnostic::json_result(task, success, reporter.diagnostics())
        );
    }
    if !success {
        std::process::exit(1);
    }
}

fn project_path(p: PathBuf) -> PathBuf {
    let p = if p.is_absolute() {
        p
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    };
    candlec::config::normalize(&p)
}
