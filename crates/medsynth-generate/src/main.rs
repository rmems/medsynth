//! CLI for the generation pipeline.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use medsynth_generate::batch::{ExecuteOptions, LaneFilter, PlanPaths, execute, load_plan};
use medsynth_generate::procedural;
use medsynth_generate::teacher::{
    HttpTeacherClient, PreparedTeacher, TeacherClient, TeacherError, TeacherPinRegistry,
};

#[derive(Parser)]
#[command(
    name = "medsynth-generate",
    version,
    about = "Generate unverified medsynth candidate records from procedural templates and pinned teachers"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a batch and write JSONL shards plus manifest.json.
    Run(RunArgs),
    /// Validate the teacher pin registry and report readiness.
    CheckPin(CheckPinArgs),
    /// Parse and check the taxonomy §5 coverage matrix.
    Matrix(MatrixArgs),
    /// List procedural template ids.
    Templates,
}

#[derive(clap::Args)]
struct RunArgs {
    /// Target-list config JSON.
    #[arg(long)]
    config: PathBuf,
    /// Output directory. Must not be under data/.
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value = "docs/taxonomy.md")]
    taxonomy: PathBuf,
    #[arg(long, default_value = "docs/sources.json")]
    sources: PathBuf,
    #[arg(long, default_value = "docs/teachers.md")]
    teachers: PathBuf,
    #[arg(long, default_value = "schemas")]
    schemas: PathBuf,
    #[arg(long, default_value = "data/gold")]
    gold: PathBuf,
    /// Override the seed in the config.
    #[arg(long)]
    seed: Option<u64>,
    /// Which lane(s) to run.
    #[arg(long, value_enum, default_value_t = LaneArg::All)]
    lane: LaneArg,
}

#[derive(Clone, Copy, ValueEnum)]
enum LaneArg {
    All,
    Procedural,
    Teacher,
}

#[derive(clap::Args)]
struct CheckPinArgs {
    #[arg(long, default_value = "docs/teachers.md")]
    teachers: PathBuf,
    /// Check a single pin id; otherwise report every pin.
    #[arg(long)]
    pin: Option<String>,
}

#[derive(clap::Args)]
struct MatrixArgs {
    #[arg(long, default_value = "docs/taxonomy.md")]
    taxonomy: PathBuf,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Run(args) => run_batch(args),
        Command::CheckPin(args) => check_pin(args),
        Command::Matrix(args) => show_matrix(args),
        Command::Templates => {
            for id in procedural::template_ids() {
                println!("{id}");
            }
            Ok(())
        }
    }
}

fn run_batch(args: RunArgs) -> Result<()> {
    let paths = PlanPaths {
        config: args.config,
        taxonomy: args.taxonomy,
        sources: args.sources,
        teachers: args.teachers,
        schemas: args.schemas,
        gold: args.gold,
    };
    let plan = load_plan(&paths, args.seed)?;
    // Refuse a `data/` output dir before any work: teacher requests burn API
    // quota, so the destination must be valid before the first request.
    medsynth_generate::batch::check_output_dir(&paths, &args.out)?;
    let env = |name: &str| std::env::var(name).ok();
    let factory = |prepared: &PreparedTeacher| -> Result<Box<dyn TeacherClient>, TeacherError> {
        Ok(Box::new(HttpTeacherClient::new(prepared)?))
    };
    let options = ExecuteOptions {
        lane_filter: match args.lane {
            LaneArg::All => LaneFilter::All,
            LaneArg::Procedural => LaneFilter::Procedural,
            LaneArg::Teacher => LaneFilter::Teacher,
        },
        env: &env,
        teacher_client_factory: &factory,
    };
    let output = execute(&plan, &options)?;
    let written = output.write_output(&args.out)?;
    for path in &written {
        println!("wrote {}", path.display());
    }
    if let Some(error) = output.lane_error {
        return Err(anyhow::Error::new(error).context(
            "procedural output was written, but the teacher lane did not run; see the manifest",
        ));
    }
    Ok(())
}

fn check_pin(args: CheckPinArgs) -> Result<()> {
    let registry = TeacherPinRegistry::load(&args.teachers)
        .with_context(|| format!("cannot load {}", args.teachers.display()))?;
    if let Some(pin_id) = args.pin {
        registry.ensure_teacher(&pin_id)?;
        println!("{pin_id}: teacher-ready");
        return Ok(());
    }
    for pin in registry.pins() {
        match registry.ensure_teacher(&pin.pin_id) {
            Ok(_) => println!("{}: teacher-ready ({})", pin.pin_id, pin.variant),
            Err(error) => println!("{}: NOT READY: {error}", pin.pin_id),
        }
    }
    Ok(())
}

fn show_matrix(args: MatrixArgs) -> Result<()> {
    let matrix = medsynth_generate::CoverageMatrix::load(&args.taxonomy)
        .with_context(|| format!("cannot load {}", args.taxonomy.display()))?;
    println!("taxonomy version: {}", matrix.version);
    println!("total target: {}", matrix.total());
    for (cell, target) in matrix.cells() {
        println!("{}: {target}", cell.label());
    }
    Ok(())
}
