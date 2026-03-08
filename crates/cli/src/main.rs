mod commands;
mod telemetry;

use clap::{Parser, Subcommand};

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum LogFormat {
    Pretty,
    Json,
}

impl From<LogFormat> for telemetry::RuntimeLogFormat {
    fn from(value: LogFormat) -> Self {
        match value {
            LogFormat::Pretty => telemetry::RuntimeLogFormat::Pretty,
            LogFormat::Json => telemetry::RuntimeLogFormat::Json,
        }
    }
}

#[derive(Parser)]
#[command(name = "thunder", version, about = "Deno-based edge function runtime")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose logging (RUST_LOG=debug)
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Runtime log format.
    #[arg(
        long,
        value_enum,
        default_value = "pretty",
        global = true,
        env = "EDGE_RUNTIME_LOG_FORMAT"
    )]
    log_format: LogFormat,

    #[command(flatten)]
    telemetry: telemetry::TelemetryArgs,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the edge runtime server
    Start(commands::start::StartArgs),
    /// Bundle a TypeScript/JavaScript file into an eszip
    Bundle(commands::bundle::BundleArgs),
    /// Watch directory for TypeScript/JavaScript functions
    Watch(commands::watch::WatchArgs),
    /// Run JavaScript/TypeScript compatibility tests inside the runtime
    Test(commands::test::TestArgs),
    /// Typecheck TypeScript/JavaScript source files (delegates to deno check)
    Check(commands::check::CheckArgs),
}

fn main() -> Result<(), anyhow::Error> {
    let cli = Cli::parse();

    // Required by deno_fetch/deno_net TLS operations (e.g. EventSource over HTTPS).
    let _ = rustls::crypto::ring::default_provider().install_default();

    telemetry::init(cli.verbose, cli.log_format.into(), &cli.telemetry)?;

    // Initialize V8 platform (must be done on main thread, before any JsRuntime)
    deno_core::JsRuntime::init_platform(None);

    let result = match cli.command {
        Commands::Start(args) => commands::start::run(args),
        Commands::Bundle(args) => commands::bundle::run(args),
        Commands::Watch(args) => commands::watch::run(args),
        Commands::Test(args) => commands::test::run(args),
        Commands::Check(args) => commands::check::run(args),
    };

    telemetry::shutdown();
    result
}
