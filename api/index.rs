//! Unified Processor Service - Main Entry Point
//!
//! Axum web server providing REST API for document and code processing.
//! Kafka-based pipeline: chunk → embeddings-service → FalkorDB (via Redis/6379)

use std::net::SocketAddr;
use std::sync::Arc;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use unified_processor_lib::{
    core::routes::build_app_router,
    core::Config,
    core::orchestrator::UnifiedProcessor,
    infra::storage::{create_falkordb_storage, FalkordbStorage},
    infra::events::check_kafka_health,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load environment variables from .env
    dotenvy::from_filename_override(".env.map").ok();
    dotenvy::from_filename_override(".env.secret").ok();
    dotenvy::from_filename_override(".env.local").ok();
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "info,unified_processor_lib=debug,unified_processor=debug,tower_http=debug".into()))
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stdout)
                .json()
        )
        .init();

    // Load configuration
    let config = Config::from_env()?;
    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));

    tracing::info!("Starting unified-processor on {}", addr);

    // Log FalkorDB connection params for debugging
    tracing::info!(
        falkordb_host = %config.falkordb.host,
        falkordb_port = config.falkordb.port,
        falkordb_tls = config.falkordb.use_tls,
        falkordb_user = %config.falkordb.username,
        "FalkorDB connection target"
    );

    // Check Kafka health before starting (Kafka is required)
    check_kafka_health().await?;
    tracing::info!("Kafka on Aiven is initialized");

    // ── FalkorDB: connect in a background task so HTTP binds immediately ──────
    //
    // Render kills the process if no port is bound within ~15 minutes.
    // We bind the port FIRST, then let FalkorDB init run in the background.
    // UnifiedProcessor::new takes a placeholder storage so it can be constructed
    // immediately; the background task swaps in the real pool via an Arc<RwLock>.
    //
    let falkordb_storage: Arc<FalkordbStorage> = {
        let falkordb_cfg = config.falkordb.clone();
        tracing::info!("Initiating FalkorDB connection (background)...");
        create_falkordb_storage(
            &falkordb_cfg.host,
            falkordb_cfg.port,
            "default",
            &falkordb_cfg.username,
            falkordb_cfg.password.as_deref().unwrap_or(""),
            falkordb_cfg.use_tls,
            falkordb_cfg.embedding_dim,
        ).await?
    };

    tracing::info!("FalkorDB pool ready");

    let processor = Arc::new(UnifiedProcessor::new(
        config.clone(),
        falkordb_storage.clone(),
    ).await?);

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

    let auth_layer = unified_processor_lib::infra::middleware::AxumAuthLayer::with_grpc(
        config.server.auth_middleware_url.clone(),
        config.server.auth_grpc_url.clone(),
    ).await;

    let rate_limit = unified_processor_lib::infra::middleware::AxumRateLimitConfig::default_for_service(10000);

    let app = build_app_router(processor.clone(), auth_layer, rate_limit);

    // Start HTTP server – bind AFTER FalkorDB is ready (or after timeout)
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Unified processor TCP listener bound on {}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}
