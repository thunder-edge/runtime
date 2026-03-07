mod commands;

use clap::{Parser, Subcommand};

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum LogFormat {
    Pretty,
    Json,
}

#[derive(Parser)]
#[command(
    name = "deno-edge-runtime",
    version,
    about = "Deno-based edge function runtime"
)]
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

    // Initialize tracing
    let env_filter = if cli.verbose { "debug" } else { "info" };
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(env_filter));

    match cli.log_format {
        LogFormat::Pretty => {
            tracing_subscriber::fmt().with_env_filter(env_filter).init();
        }
        LogFormat::Json => {
            tracing_subscriber::fmt()
                .json()
                .with_current_span(true)
                .with_span_list(false)
                .with_env_filter(env_filter)
                .init();
        }
    }

    // Initialize V8 platform (must be done on main thread, before any JsRuntime)
    deno_core::JsRuntime::init_platform(None);

    match cli.command {
        Commands::Start(args) => commands::start::run(args),
        Commands::Bundle(args) => commands::bundle::run(args),
        Commands::Watch(args) => commands::watch::run(args),
        Commands::Test(args) => commands::test::run(args),
        Commands::Check(args) => commands::check::run(args),
    }
}
