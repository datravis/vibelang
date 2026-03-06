mod ast;
mod codegen;
mod lexer;
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
        /// Target triple (default: aarch64-apple-darwin)
        #[arg(long, default_value = "aarch64-apple-darwin")]
        target: String,
        /// Emit LLVM IR instead of object code
        #[arg(long)]
        emit_ir: bool,
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
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Build {
            file,
            output,
            target,
            emit_ir,
        } => run_build(&file, output.as_deref(), &target, emit_ir),
        Command::Check { file } => run_check(&file),
        Command::Run { file } => run_run(&file),
        Command::Lex { file } => run_lex(&file),
        Command::Parse { file } => run_parse(&file),
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
) -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(file)?;
    let tokens = lexer::lex(&source)?;
    let module = parser::parse(tokens)?;
    types::check(&module)?;

    let default_output = file.with_extension(if emit_ir { "ll" } else { "o" });
    let out_path = output.unwrap_or(&default_output);

    if emit_ir {
        codegen::emit_ir(&module, out_path, target)?;
    } else {
        codegen::emit_object(&module, out_path, target)?;
    }

    println!("Compiled: {}", out_path.display());
    Ok(())
}

fn run_run(file: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string(file)?;
    let tokens = lexer::lex(&source)?;
    let module = parser::parse(tokens)?;
    types::check(&module)?;
    codegen::jit_run(&module)?;
    Ok(())
}
