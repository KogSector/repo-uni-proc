//! Unified Processor Service - Main Entry Point
//!
//! Startup order (critical for Render deployments):
//!   1. Tracing + config
//!   2. Kafka health check (fast, required)
//!   3. ── Bind TCP port ──  ← Render sees the service live immediately
//!   4. FalkorDB pool (lazy – smoke-test capped at 10 s total)
//!   5. Build processor + middleware
//!   6. Kafka consumer (background task)
//!   7. axum::serve

use std::net::SocketAddr;
use std::sync::Arc;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use unified_processor_lib::{
    core::routes::build_app_router,
    core::Config,
    core::orchestrator::UnifiedProcessor,
    infra::storage::create_falkordb_storage,
    infra::events::check_kafka_health,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Environment variables ────────────────────────────────────────────────
    dotenvy::from_filename_override(".env.map").ok();
    dotenvy::from_filename_override(".env.secret").ok();
    dotenvy::from_filename_override(".env.local").ok();

    // ── Tracing ─────────────────────────────────────────────────────────────
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "info,unified_processor_lib=debug,unified_processor=debug,tower_http=debug".into()))
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stdout)
                .json()
        )
        .init();

    // ── Config ───────────────────────────────────────────────────────────────
    let config = Config::from_env().unwrap_or_else(|e| {
        tracing::warn!("Config error, using defaults: {}", e);
        // Provide minimal default config for Render deployment
        // IMPORTANT: These defaults will NOT work for actual processing
        // You MUST set environment variables in Render dashboard
        Config {
            server: unified_processor_lib::core::config::ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 8090,
                workers: 4,
                grpc_host: "0.0.0.0".to_string(),
                grpc_port: 50052,
                auth_middleware_url: "http://auth-middleware:8080".to_string(),
                auth_grpc_url: "".to_string(),
            },
            database: unified_processor_lib::core::config::DatabaseConfig {
                DATABASE_URL: "postgresql://user:password@localhost:5432/dbname".to_string(),
                max_connections: 20,
            },
            pipeline: unified_processor_lib::core::config::PipelineConfig {
                max_file_size: 10485760,
                chunk_size: 1000,
                max_batch_size: 100,
                timeout: std::time::Duration::from_secs(300),
            },
            grpc: Some(unified_processor_lib::core::config::GrpcConfig {
                host: "0.0.0.0".to_string(),
                port: 50052,
            }),
            falkordb: unified_processor_lib::core::config::FalkordbConfig {
                host: "localhost".to_string(),
                port: 50860,
                username: "falkordb".to_string(),
                password: Some("adminconfuse".to_string()),
                use_tls: false,
                embedding_dim: 1536,
                timeout_secs: 30,
            },
            kafka: unified_processor_lib::core::config::KafkaConfig {
                bootstrap_servers: "localhost:9092".to_string(),
                group_id: "repo-uni-proc-group".to_string(),
                client_id: "unified-processor".to_string(),
                auto_offset_reset: "earliest".to_string(),
                enable_auto_commit: true,
            },
            web: unified_processor_lib::core::config::WebConfig {
                enabled: true,
                max_pages: 50,
                max_depth: 3,
                crawl_delay_ms: 1000,
                user_agent: "UnifiedProcessorBot/1.0".to_string(),
                request_timeout_secs: 30,
                max_concurrent_crawls: 5,
            },
        }
    });

    // Check if we're using default config (indicates missing env vars)
    if config.database.DATABASE_URL.contains("localhost") || config.database.DATABASE_URL.contains("user:password") {
        tracing::error!("CRITICAL: DATABASE_URL environment variable not set or using default. Please set DATABASE_URL in Render dashboard.");
        tracing::error!("Service will not function properly without real database connection.");
    }

    if config.falkordb.host == "localhost" {
        tracing::error!("CRITICAL: FALKORDB_HOST environment variable not set or using default. Please set FALKORDB_HOST in Render dashboard.");
        tracing::error!("Service will not function properly without real FalkorDB connection.");
    }

    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));

    tracing::info!(
        port = config.server.port,
        falkordb_host = %config.falkordb.host,
        falkordb_port = config.falkordb.port,
        falkordb_tls  = config.falkordb.use_tls,
        falkordb_user = %config.falkordb.username,
        "Starting unified-processor"
    );

    // ── Step 1: Bind TCP port FIRST ──────────────────────────────────────────
    // Render kills a deploy if no port is detected within ~15 minutes.
    // Binding here guarantees Render sees us as live immediately even if downstream checks retry.
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(bound_addr = %addr, "TCP listener bound — service accepting connections");

    // ── Step 2: Kafka health check ───────────────────────────────────────────
    if let Err(e) = check_kafka_health().await {
        tracing::warn!("Kafka health check warning (proceeding with HTTP startup): {}", e);
    } else {
        tracing::info!("Kafka on Aiven is initialized");
    }

    // ── Step 3: FalkorDB connection pool ─────────────────────────────────────
    // create_falkordb_storage builds a lazy bb8 pool with an 8 s connection
    // timeout and runs a single smoke-test (hard-capped at 10 s).  If the
    // smoke-test fails, it logs a warning and returns the pool anyway — actual
    // operations will surface errors per-request instead of crashing startup.
    let falkordb_storage = create_falkordb_storage(
        &config.falkordb.host,
        config.falkordb.port,
        "",
        &config.falkordb.username,
        config.falkordb.password.as_deref().unwrap_or(""),
        config.falkordb.use_tls,
        config.falkordb.embedding_dim,
    ).await?;

    // ── Step 4: Processor ────────────────────────────────────────────────────
    let processor = Arc::new(UnifiedProcessor::new(
        config.clone(),
        falkordb_storage.clone(),
    ).await?);

    // ── Step 5: Kafka consumer (background) ──────────────────────────────────
    #[cfg(feature = "kafka")]
    {
        let consumer_processor = processor.clone();
        tokio::spawn(async move {
            tracing::info!("Initializing Kafka event consumer...");
            let consumer = unified_processor_lib::graph::consumer::UnifiedEventConsumer::new(consumer_processor);
            if let Err(e) = consumer.start().await {
                tracing::error!("Kafka consumer failed to start: {}", e);
            }
        });
    }

    // ── Step 5.5: FalkorDB keep-alive (background) ─────────────────────────────
    // Keep FalkorDB active by pinging it every 4 minutes to prevent free tier spin-down
    let keepalive_pool = falkordb_storage.get_pool().clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(240)); // 4 minutes
        loop {
            interval.tick().await;
            match keepalive_pool.get().await {
                Ok(mut conn) => {
                    match redis::cmd("PING").query_async::<_, String>(&mut *conn).await {
                        Ok(_) => {
                            tracing::debug!("FalkorDB keep-alive ping successful");
                        }
                        Err(e) => {
                            tracing::warn!("FalkorDB keep-alive ping failed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("FalkorDB keep-alive connection failed: {}", e);
                }
            }
        }
    });

    // ── Step 6: Router + middleware ──────────────────────────────────────────
    let auth_layer = unified_processor_lib::infra::middleware::AxumAuthLayer::with_grpc(
        config.server.auth_middleware_url.clone(),
        config.server.auth_grpc_url.clone(),
    ).await;

    let rate_limit = unified_processor_lib::infra::middleware::AxumRateLimitConfig::default_for_service(10000);

    let app = build_app_router(processor.clone(), auth_layer, rate_limit);

    // ── Step 7: Serve ────────────────────────────────────────────────────────
    tracing::info!(addr = %addr, "Serving");
    axum::serve(listener, app).await?;

    Ok(())
}
