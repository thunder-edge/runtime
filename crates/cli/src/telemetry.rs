use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use clap::{Args, ValueEnum};
use opentelemetry::logs::{AnyValue, LogRecord as _, Logger as _, LoggerProvider as _, Severity};
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::logs::LoggerProvider;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::TracerProvider;
use opentelemetry_sdk::Resource;
use runtime_core::isolate_logs::{drain_collected_logs, IsolateConsoleLog};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum RuntimeLogFormat {
    Pretty,
    Json,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum OtlpProtocol {
    HttpProtobuf,
}

#[derive(Debug, Args, Clone)]
pub struct TelemetryArgs {
    /// Enable OpenTelemetry exporters (traces/metrics/logs).
    #[arg(
        long,
        default_value_t = false,
        global = true,
        env = "EDGE_RUNTIME_OTEL_ENABLED"
    )]
    otel_enabled: bool,

    /// OTLP transport protocol.
    #[arg(
        long,
        value_enum,
        default_value = "http-protobuf",
        global = true,
        env = "EDGE_RUNTIME_OTEL_PROTOCOL"
    )]
    otel_protocol: OtlpProtocol,

    /// OTLP collector base endpoint.
    #[arg(
        long,
        default_value = "http://127.0.0.1:4318",
        global = true,
        env = "EDGE_RUNTIME_OTEL_ENDPOINT"
    )]
    otel_endpoint: String,

    /// OTEL service.name resource attribute.
    #[arg(
        long,
        default_value = "thunder",
        global = true,
        env = "EDGE_RUNTIME_OTEL_SERVICE_NAME"
    )]
    otel_service_name: String,

    /// Export interval in milliseconds for periodic exporters.
    #[arg(
        long,
        default_value_t = 5000,
        global = true,
        env = "EDGE_RUNTIME_OTEL_EXPORT_INTERVAL_MS"
    )]
    otel_export_interval_ms: u64,

    /// Export timeout in milliseconds.
    #[arg(
        long,
        default_value_t = 10000,
        global = true,
        env = "EDGE_RUNTIME_OTEL_EXPORT_TIMEOUT_MS"
    )]
    otel_export_timeout_ms: u64,

    /// Enable OTEL trace signal export.
    #[arg(
        long,
        default_value_t = true,
        global = true,
        env = "EDGE_RUNTIME_OTEL_ENABLE_TRACES"
    )]
    otel_enable_traces: bool,

    /// Enable OTEL metrics signal export.
    #[arg(
        long,
        default_value_t = true,
        global = true,
        env = "EDGE_RUNTIME_OTEL_ENABLE_METRICS"
    )]
    otel_enable_metrics: bool,

    /// Enable OTEL logs signal export.
    #[arg(
        long,
        default_value_t = true,
        global = true,
        env = "EDGE_RUNTIME_OTEL_ENABLE_LOGS"
    )]
    otel_enable_logs: bool,

    /// Export isolate collector logs to OTEL logs signal.
    #[arg(
        long,
        default_value_t = true,
        global = true,
        env = "EDGE_RUNTIME_OTEL_EXPORT_ISOLATE_LOGS"
    )]
    otel_export_isolate_logs: bool,

    /// Max isolate logs drained per exporter tick.
    #[arg(
        long,
        default_value_t = 256,
        global = true,
        env = "EDGE_RUNTIME_OTEL_ISOLATE_LOG_BATCH_SIZE"
    )]
    otel_isolate_log_batch_size: usize,
}

struct IsolateLogOtelBridge {
    logger_provider: LoggerProvider,
    exported_counter: opentelemetry::metrics::Counter<u64>,
    error_counter: opentelemetry::metrics::Counter<u64>,
    batch_histogram: opentelemetry::metrics::Histogram<u64>,
    interval: Duration,
    batch_size: usize,
}

struct TelemetryState {
    otel_enabled: bool,
    isolate_log_export_enabled: bool,
    tracer_provider: Option<TracerProvider>,
    meter_provider: Option<SdkMeterProvider>,
    logger_provider: Option<LoggerProvider>,
    isolate_bridge: Option<IsolateLogOtelBridge>,
}

impl TelemetryState {
    fn disabled() -> Self {
        Self {
            otel_enabled: false,
            isolate_log_export_enabled: false,
            tracer_provider: None,
            meter_provider: None,
            logger_provider: None,
            isolate_bridge: None,
        }
    }
}

static TELEMETRY_STATE: OnceLock<Mutex<TelemetryState>> = OnceLock::new();

fn telemetry_state() -> &'static Mutex<TelemetryState> {
    TELEMETRY_STATE.get_or_init(|| Mutex::new(TelemetryState::disabled()))
}

pub fn init(
    verbose: bool,
    log_format: RuntimeLogFormat,
    args: &TelemetryArgs,
) -> Result<(), anyhow::Error> {
    let env_filter = if verbose { "debug" } else { "info" };
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(env_filter));

    if !args.otel_enabled {
        init_tracing_subscriber(log_format, env_filter)?;
        if let Ok(mut guard) = telemetry_state().lock() {
            *guard = TelemetryState::disabled();
        }
        return Ok(());
    }

    match args.otel_protocol {
        OtlpProtocol::HttpProtobuf => {}
    }

    let resource = Resource::new(vec![KeyValue::new(
        "service.name",
        args.otel_service_name.clone(),
    )]);

    let timeout = Duration::from_millis(args.otel_export_timeout_ms);
    let interval = Duration::from_millis(args.otel_export_interval_ms);

    let mut tracer_provider = None;
    if args.otel_enable_traces {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(format!(
                "{}/v1/traces",
                args.otel_endpoint.trim_end_matches('/')
            ))
            .with_timeout(timeout)
            .build()?;

        let provider = TracerProvider::builder()
            .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
            .with_resource(resource.clone())
            .build();

        let tracer = provider.tracer("thunder");
        global::set_tracer_provider(provider.clone());
        tracer_provider = Some(provider);

        match log_format {
            RuntimeLogFormat::Pretty => {
                tracing_subscriber::registry()
                    .with(env_filter.clone())
                    .with(tracing_opentelemetry::layer().with_tracer(tracer))
                    .with(tracing_subscriber::fmt::layer())
                    .try_init()?;
            }
            RuntimeLogFormat::Json => {
                tracing_subscriber::registry()
                    .with(env_filter.clone())
                    .with(tracing_opentelemetry::layer().with_tracer(tracer))
                    .with(
                        tracing_subscriber::fmt::layer()
                            .json()
                            .with_current_span(true)
                            .with_span_list(false),
                    )
                    .try_init()?;
            }
        }
    } else {
        init_tracing_subscriber(log_format, env_filter)?;
    }

    let mut meter_provider = None;
    let mut isolate_metric_instruments = None;
    if args.otel_enable_metrics {
        let exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(format!(
                "{}/v1/metrics",
                args.otel_endpoint.trim_end_matches('/')
            ))
            .with_timeout(timeout)
            .build()?;

        let reader = PeriodicReader::builder(exporter, opentelemetry_sdk::runtime::Tokio)
            .with_interval(interval)
            .with_timeout(timeout)
            .build();

        let provider = SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(resource.clone())
            .build();
        global::set_meter_provider(provider.clone());

        let meter = provider.meter("thunder");
        isolate_metric_instruments = Some((
            meter
                .u64_counter("edge_runtime_isolate_logs_exported_total")
                .with_description("Total number of isolate console logs exported via OTEL")
                .build(),
            meter
                .u64_counter("edge_runtime_isolate_log_export_errors_total")
                .with_description(
                    "Total number of isolate console logs dropped due to export errors",
                )
                .build(),
            meter
                .u64_histogram("edge_runtime_isolate_log_batch_size")
                .with_description("Isolate log export batch sizes")
                .build(),
        ));

        meter_provider = Some(provider);
    }

    let mut logger_provider = None;
    if args.otel_enable_logs {
        let exporter = opentelemetry_otlp::LogExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(format!(
                "{}/v1/logs",
                args.otel_endpoint.trim_end_matches('/')
            ))
            .with_timeout(timeout)
            .build()?;

        let provider = LoggerProvider::builder()
            .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
            .with_resource(resource)
            .build();

        logger_provider = Some(provider);
    }

    let isolate_bridge = if args.otel_export_isolate_logs {
        match (logger_provider.clone(), isolate_metric_instruments) {
            (Some(logger_provider), Some((exported_counter, error_counter, batch_histogram))) => {
                Some(IsolateLogOtelBridge {
                    logger_provider,
                    exported_counter,
                    error_counter,
                    batch_histogram,
                    interval,
                    batch_size: args.otel_isolate_log_batch_size.max(1),
                })
            }
            (Some(logger_provider), None) => {
                let meter = global::meter("thunder");
                Some(IsolateLogOtelBridge {
                    logger_provider,
                    exported_counter: meter
                        .u64_counter("edge_runtime_isolate_logs_exported_total")
                        .build(),
                    error_counter: meter
                        .u64_counter("edge_runtime_isolate_log_export_errors_total")
                        .build(),
                    batch_histogram: meter
                        .u64_histogram("edge_runtime_isolate_log_batch_size")
                        .build(),
                    interval,
                    batch_size: args.otel_isolate_log_batch_size.max(1),
                })
            }
            _ => None,
        }
    } else {
        None
    };

    if let Ok(mut guard) = telemetry_state().lock() {
        *guard = TelemetryState {
            otel_enabled: true,
            isolate_log_export_enabled: args.otel_export_isolate_logs,
            tracer_provider,
            meter_provider,
            logger_provider,
            isolate_bridge,
        };
    }

    info!(
        function_name = "runtime",
        request_id = "system",
        otel_endpoint = %args.otel_endpoint,
        traces = args.otel_enable_traces,
        metrics = args.otel_enable_metrics,
        logs = args.otel_enable_logs,
        "OpenTelemetry initialized"
    );

    Ok(())
}

fn init_tracing_subscriber(
    log_format: RuntimeLogFormat,
    env_filter: tracing_subscriber::EnvFilter,
) -> Result<(), anyhow::Error> {
    match log_format {
        RuntimeLogFormat::Pretty => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_subscriber::fmt::layer())
                .try_init()?;
        }
        RuntimeLogFormat::Json => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(
                    tracing_subscriber::fmt::layer()
                        .json()
                        .with_current_span(true)
                        .with_span_list(false),
                )
                .try_init()?;
        }
    }

    Ok(())
}

pub fn spawn_isolate_log_exporter(shutdown: CancellationToken, print_isolate_logs: bool) {
    let bridge = {
        let Ok(mut guard) = telemetry_state().lock() else {
            return;
        };

        if !guard.otel_enabled || !guard.isolate_log_export_enabled {
            return;
        }

        guard.isolate_bridge.take()
    };

    let Some(bridge) = bridge else {
        return;
    };

    if print_isolate_logs {
        warn!(
            function_name = "runtime",
            request_id = "system",
            "OTEL isolate log export enabled but --print-isolate-logs=true; collector path will stay empty"
        );
        return;
    }

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(bridge.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    debug!(function_name = "runtime", request_id = "system", "stopping isolate log OTEL exporter worker");
                    break;
                }
                _ = ticker.tick() => {
                    let batch = drain_collected_logs(bridge.batch_size);
                    if batch.is_empty() {
                        continue;
                    }

                    let mut exported = 0_u64;
                    for entry in &batch {
                        match emit_isolate_log(&bridge.logger_provider, entry) {
                            Ok(()) => {
                                exported += 1;
                            }
                            Err(err) => {
                                bridge.error_counter.add(1, &[]);
                                error!(
                                    function_name = %entry.function_name,
                                    request_id = %entry.request_id,
                                    "failed to emit isolate log to OTEL provider: {}",
                                    err
                                );
                            }
                        }
                    }

                    if exported > 0 {
                        bridge.exported_counter.add(exported, &[]);
                        bridge.batch_histogram.record(exported, &[]);
                    }
                }
            }
        }
    });
}

fn emit_isolate_log(
    logger_provider: &LoggerProvider,
    entry: &IsolateConsoleLog,
) -> Result<(), anyhow::Error> {
    let logger = logger_provider.logger("thunder-isolate");
    let mut record = logger.create_log_record();

    let (severity_number, severity_text) = match entry.level {
        0 => (Severity::Info, "INFO"),
        1 => (Severity::Warn, "WARN"),
        _ => (Severity::Error, "ERROR"),
    };

    record.set_severity_number(severity_number);
    record.set_severity_text(severity_text);
    record.set_body(AnyValue::from(entry.message.clone()));
    record.add_attributes([
        ("function_name", entry.function_name.clone()),
        ("request_id", entry.request_id.clone()),
        ("log_source", "isolate".to_string()),
        ("timestamp", entry.timestamp.to_rfc3339()),
    ]);

    logger.emit(record);
    Ok(())
}

pub fn shutdown() {
    let Ok(mut guard) = telemetry_state().lock() else {
        return;
    };

    if let Some(provider) = guard.logger_provider.take() {
        let _ = provider.shutdown();
    }
    if let Some(provider) = guard.meter_provider.take() {
        let _ = provider.shutdown();
    }
    if let Some(provider) = guard.tracer_provider.take() {
        let _ = provider.shutdown();
    }

    guard.isolate_bridge = None;
    guard.otel_enabled = false;
    guard.isolate_log_export_enabled = false;
}
