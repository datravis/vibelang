#![allow(dead_code)]

mod ast;
mod codegen;
mod lexer;
mod memory;
mod parser;
mod types;

use clap::{Parser as ClapParser, Subcommand};
use std::path::PathBuf;
use std::process;

#[derive(ClapParser)]
#[command(name = "vibe", about = "The VibeLang compiler", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compile a VibeLang source file
    Build {
        /// Source file to compile
        file: PathBuf,
        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Target triple (aarch64-apple-darwin, x86_64-unknown-linux-gnu, x86_64-pc-windows-msvc)
        #[arg(long, default_value = "aarch64-apple-darwin")]
        target: String,
        /// Emit LLVM IR instead of object code
        #[arg(long)]
        emit_ir: bool,
        /// Optimization level (0=none, 1=basic, 2=default, 3=aggressive)
        #[arg(short = 'O', long = "opt-level", default_value = "2")]
        opt_level: u8,
    },
    /// Type-check a VibeLang source file without compiling
    Check {
        /// Source file to check
        file: PathBuf,
    },
    /// Compile and run a VibeLang source file
    Run {
        /// Source file to run
        file: PathBuf,
        /// Optimization level (0=none, 1=basic, 2=default, 3=aggressive)
        #[arg(short = 'O', long = "opt-level", default_value = "2")]
        opt_level: u8,
    },
    /// Lex a file and print tokens (debug)
    Lex {
        /// Source file to lex
        file: PathBuf,
    },
    /// Parse a file and print AST (debug)
    Parse {
        /// Source file to parse
        file: PathBuf,
    },
    /// List supported compilation targets
    Targets,
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Build {
            file,
            output,
            target,
            emit_ir,
            opt_level,
        } => run_build(&file, output.as_deref(), &target, emit_ir, opt_level),
        Command::Check { file } => run_check(&file),
        Command::Run { file, opt_level } => run_run(&file, opt_level),
        Command::Lex { file } => run_lex(&file),
        Command::Parse { file } => run_parse(&file),
        Command::Targets => run_targets(),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

fn run_lex(file: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(file)?;
    let tokens = lexer::lex(&source)?;
    for tok in &tokens {
        println!("{tok:?}");
    }
    Ok(())
}

fn run_parse(file: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(file)?;
    let tokens = lexer::lex(&source)?;
    let module = parser::parse(tokens)?;
    println!("{module:#?}");
    Ok(())
}

fn run_check(file: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(file)?;
    let tokens = lexer::lex(&source)?;
    let module = parser::parse(tokens)?;
    types::check(&module)?;
    println!("OK: type check passed");
    Ok(())
}

fn run_build(
    file: &std::path::Path,
    output: Option<&std::path::Path>,
    target: &str,
    emit_ir: bool,
    opt_level: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(file)?;
    let tokens = lexer::lex(&source)?;
    let module = parser::parse(tokens)?;
    types::check(&module)?;

    let default_output = file.with_extension(if emit_ir { "ll" } else { "o" });
    let out_path = output.unwrap_or(&default_output);

    if emit_ir {
        codegen::emit_ir(&module, out_path, target, opt_level)?;
    } else {
        codegen::emit_object(&module, out_path, target, opt_level)?;
    }

    println!("Compiled: {}", out_path.display());
    Ok(())
}

fn run_targets() -> Result<(), Box<dyn std::error::Error>> {
    println!("Supported compilation targets:");
    println!();
    println!("  aarch64-apple-darwin      Apple Silicon (M1/M2/M3) macOS");
    println!("  x86_64-unknown-linux-gnu  Linux x86-64 (glibc)");
    println!("  x86_64-pc-windows-msvc    Windows x86-64 (MSVC ABI)");
    println!();
    println!("Usage: vibe build --target <TARGET> <file>");
    Ok(())
}

fn run_run(file: &std::path::Path, opt_level: u8) -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(file)?;
    let tokens = lexer::lex(&source)?;
    let module = parser::parse(tokens)?;
    types::check(&module)?;
    codegen::jit_run(&module, opt_level)?;
    Ok(())
}
