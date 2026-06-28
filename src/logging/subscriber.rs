//! Shared tracing subscriber initialization with consistent redaction and formatting.

use std::sync::Arc;

use tracing::Level;
use tracing_subscriber::{fmt, prelude::*, reload, EnvFilter, Registry};

use crate::log_scrub::RedactingFields;
use crate::logging::{analytics::AnalyticsEngine, sampling::SamplingConfig, AnalyticsLayer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogOutputFormat {
    Json,
    Pretty,
}

pub struct SubscriberConfig {
    pub level: Level,
    pub format: LogOutputFormat,
    pub analytics: bool,
    pub reload_handle: bool,
    pub otel: bool,
}

impl Default for SubscriberConfig {
    fn default() -> Self {
        Self {
            level: Level::INFO,
            format: LogOutputFormat::Json,
            analytics: false,
            reload_handle: false,
            otel: false,
        }
    }
}

impl SubscriberConfig {
    pub fn from_level_str(level: &str, format: LogOutputFormat) -> Self {
        Self {
            level: level.parse().unwrap_or(Level::INFO),
            format,
            ..Default::default()
        }
    }
}

pub struct SubscriberGuard {
    pub reload_handle: Option<reload::Handle<EnvFilter, Registry>>,
}

pub struct SubscriberInit {
    pub guard: SubscriberGuard,
    pub analytics_engine: Option<Arc<AnalyticsEngine>>,
}

fn env_filter_for(config: &SubscriberConfig) -> EnvFilter {
    EnvFilter::builder()
        .with_default_directive(config.level.into())
        .from_env_lossy()
}

fn analytics_engine_for(config: &SubscriberConfig) -> Option<Arc<AnalyticsEngine>> {
    if config.analytics {
        Some(Arc::new(AnalyticsEngine::new(std::time::Duration::from_secs(
            3600,
        ))))
    } else {
        None
    }
}

fn otel_enabled(config: &SubscriberConfig) -> bool {
    config.otel && std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok()
}

fn init_simple(config: &SubscriberConfig) {
    let env_filter = env_filter_for(config);
    let redacting = RedactingFields::new();
    match config.format {
        LogOutputFormat::Json => {
            let fmt_layer = fmt::layer().json().with_target(true).fmt_fields(redacting);
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .init();
        }
        LogOutputFormat::Pretty => {
            let fmt_layer = fmt::layer().pretty().with_target(true).fmt_fields(redacting);
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .init();
        }
    }
}

fn init_operator_stack(
    config: &SubscriberConfig,
    analytics_engine: &Arc<AnalyticsEngine>,
    reload_handle_out: &mut Option<reload::Handle<EnvFilter, Registry>>,
) {
    let env_filter = env_filter_for(config);
    let (filter_layer, reload_handle) = reload::Layer::new(env_filter);
    *reload_handle_out = Some(reload_handle);
    let analytics_layer =
        AnalyticsLayer::new(SamplingConfig::default(), Arc::clone(analytics_engine));
    let use_otel = otel_enabled(config);
    let redacting = RedactingFields::new();

    match config.format {
        LogOutputFormat::Json => {
            let fmt_layer = fmt::layer().json().with_target(true).fmt_fields(redacting);
            let registry = tracing_subscriber::registry()
                .with(filter_layer)
                .with(analytics_layer)
                .with(fmt_layer);
            if use_otel {
                let otel_layer = crate::telemetry::init_telemetry(&registry);
                let trace_id_layer = crate::telemetry::trace_id_layer();
                registry.with(otel_layer).with(trace_id_layer).init();
            } else {
                registry.init();
            }
        }
        LogOutputFormat::Pretty => {
            let fmt_layer = fmt::layer().pretty().with_target(true).fmt_fields(redacting);
            let registry = tracing_subscriber::registry()
                .with(filter_layer)
                .with(analytics_layer)
                .with(fmt_layer);
            if use_otel {
                let otel_layer = crate::telemetry::init_telemetry(&registry);
                let trace_id_layer = crate::telemetry::trace_id_layer();
                registry.with(otel_layer).with(trace_id_layer).init();
            } else {
                registry.init();
            }
        }
    }
}

pub fn init_subscriber(config: SubscriberConfig) -> SubscriberInit {
    let analytics_engine = analytics_engine_for(&config);
    let mut reload_handle = None;

    if config.reload_handle {
        let engine = analytics_engine
            .clone()
            .unwrap_or_else(|| Arc::new(AnalyticsEngine::new(std::time::Duration::from_secs(3600))));
        init_operator_stack(&config, &engine, &mut reload_handle);
        return SubscriberInit {
            guard: SubscriberGuard { reload_handle },
            analytics_engine: Some(engine),
        };
    }

    if config.analytics {
        let engine = analytics_engine.unwrap_or_else(|| {
            Arc::new(AnalyticsEngine::new(std::time::Duration::from_secs(3600)))
        });
        let analytics_layer = AnalyticsLayer::new(SamplingConfig::default(), Arc::clone(&engine));
        let env_filter = env_filter_for(&config);
        let redacting = RedactingFields::new();
        match config.format {
            LogOutputFormat::Json => {
                let fmt_layer = fmt::layer().json().with_target(true).fmt_fields(redacting);
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(analytics_layer)
                    .with(fmt_layer)
                    .init();
            }
            LogOutputFormat::Pretty => {
                let fmt_layer = fmt::layer().pretty().with_target(true).fmt_fields(redacting);
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(analytics_layer)
                    .with(fmt_layer)
                    .init();
            }
        }
        return SubscriberInit {
            guard: SubscriberGuard { reload_handle: None },
            analytics_engine: Some(engine),
        };
    }

    init_simple(&config);
    SubscriberInit {
        guard: SubscriberGuard { reload_handle: None },
        analytics_engine: None,
    }
}

pub fn init_binary_subscriber(level: Level, format: LogOutputFormat) -> SubscriberInit {
    init_subscriber(SubscriberConfig {
        level,
        format,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_uses_json_and_info() {
        let cfg = SubscriberConfig::default();
        assert_eq!(cfg.level, Level::INFO);
        assert_eq!(cfg.format, LogOutputFormat::Json);
    }

    #[test]
    fn from_level_str_parses_debug() {
        let cfg = SubscriberConfig::from_level_str("debug", LogOutputFormat::Pretty);
        assert_eq!(cfg.level, Level::DEBUG);
        assert_eq!(cfg.format, LogOutputFormat::Pretty);
    }
}
