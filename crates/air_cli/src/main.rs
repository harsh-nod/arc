use air_interp::run_main;
use air_syntax::parse_module;
use air_verify::verify_module;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "airc")]
#[command(about = "AIR command line interface (prototype)")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Parse an AIR file.
    Parse {
        /// AIR source file
        file: PathBuf,
        /// Print the parsed module
        #[arg(long)]
        print: bool,
    },
    /// Verify an AIR module.
    Verify {
        /// AIR source file
        file: PathBuf,
    },
    /// Run an AIR module using the reference interpreter.
    Run {
        /// AIR source file
        file: PathBuf,
    },
    /// Placeholder for future commands.
    #[command(alias = "opt")]
    Opt {
        file: PathBuf,
        #[arg(long)]
        passes: Option<String>,
    },
    #[command(alias = "lower")]
    Lower {
        file: PathBuf,
        #[arg(long)]
        to: Option<String>,
    },
    #[command(alias = "codegen")]
    Codegen {
        file: PathBuf,
        #[arg(long)]
        target: Option<String>,
    },
    #[command(alias = "trace")]
    Trace { file: PathBuf },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Parse { file, print } => {
            let source = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let module = parse_module(&source)?;
            if print {
                println!("{:#?}", module);
            }
        }
        Commands::Verify { file } => {
            let source = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let module = parse_module(&source)?;
            verify_module(&module)?;
            println!("verification succeeded for {}", file.display());
        }
        Commands::Run { file } => {
            let source = fs::read_to_string(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let module = parse_module(&source)?;
            verify_module(&module)?;
            match run_main(&module)? {
                Some(value) => println!("{}", value),
                None => println!("program completed without value"),
            }
        }
        Commands::Opt { .. }
        | Commands::Lower { .. }
        | Commands::Codegen { .. }
        | Commands::Trace { .. } => {
            eprintln!("command not implemented yet");
            std::process::exit(1);
        }
    }
    Ok(())
}
