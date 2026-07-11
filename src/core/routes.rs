use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::core::orchestrator::UnifiedProcessor;
use crate::infra::middleware::AxumAuthLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub fn build_app_router(
    processor: Arc<UnifiedProcessor>,
    auth_layer: AxumAuthLayer,
    rate_limit: crate::infra::middleware::AxumRateLimitConfig,
) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let protected_routes = Router::new()
        // Code analysis endpoints
        .route("/api/v1/codebase/analyze", post(analyze_code))
        .route("/api/v1/codebase/batch", post(batch_analyze_code))
        .route("/api/v1/codebase/languages", get(list_supported_languages))
        .route("/api/v1/codebase/metrics", post(get_code_metrics))
        // Integration endpoints
        .route("/api/v1/graph/sync", post(trigger_graph_sync))
        .route("/api/v1/status/{source_id}", get(get_processing_status))
        // Legacy compatibility endpoints
        .route("/api/v1/chunk", post(analyze_code))
        .layer(axum::middleware::from_fn_with_state(rate_limit.clone(), crate::infra::middleware::axum_rate_limit_middleware))
        .layer(axum::middleware::from_fn_with_state(auth_layer, crate::infra::middleware::axum_auth_middleware))
        .layer(axum::extract::DefaultBodyLimit::disable());

    Router::new()
        // Health endpoints
        .route("/", get(health_check))
        .route("/health", get(health_check))

        .merge(protected_routes)
        // Global Middleware stack
        .layer(axum::middleware::from_fn(crate::graph::correlation_middleware))
        .layer(axum::middleware::from_fn(crate::infra::middleware::security_headers_middleware))
        .layer(axum::middleware::from_fn(crate::infra::middleware::zero_trust_middleware))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(processor)
}

// ==========================================
// From codebase.rs
// ==========================================

// Codebase analysis API endpoints

#[derive(Debug, Deserialize)]
pub struct AnalyzeCodeRequest {
    pub content: String,
    pub filename: String,
    pub source_id: String,
    pub user_id: String,
}

#[derive(Debug, Serialize)]
pub struct AnalyzeCodeResponse {
    pub success: bool,
    pub data: Option<crate::core::orchestrator::ProcessedData>,
    pub error: Option<String>,
    pub processing_time_ms: u64,
}

pub async fn analyze_code(
    State(processor): State<Arc<UnifiedProcessor>>,
    Json(request): Json<AnalyzeCodeRequest>,
) -> Result<Json<AnalyzeCodeResponse>, StatusCode> {
    let start_time = std::time::Instant::now();
    
    match processor.process_file(&request.content, false, &request.filename, &request.source_id, "unknown/repo", &request.user_id).await {
        Ok(data) => {
            let processing_time = start_time.elapsed().as_millis() as u64;
            Ok(Json(AnalyzeCodeResponse {
                success: true,
                data: Some(data),
                error: None,
                processing_time_ms: processing_time,
            }))
        }
        Err(e) => {
            let processing_time = start_time.elapsed().as_millis() as u64;
            Ok(Json(AnalyzeCodeResponse {
                success: false,
                data: None,
                error: Some(e.to_string()),
                processing_time_ms: processing_time,
            }))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct BatchAnalyzeRequest {
    pub files: Vec<AnalyzeCodeRequest>,
}

#[derive(Debug, Serialize)]
pub struct BatchAnalyzeResponse {
    pub success: bool,
    pub analyzed_files: usize,
    pub failed_files: usize,
    pub results: Vec<AnalyzeCodeResponse>,
    pub total_processing_time_ms: u64,
}

pub async fn batch_analyze_code(
    State(processor): State<Arc<UnifiedProcessor>>,
    Json(request): Json<BatchAnalyzeRequest>,
) -> Result<Json<BatchAnalyzeResponse>, StatusCode> {
    let start_time = std::time::Instant::now();
    let mut results = Vec::new();
    let mut analyzed_count = 0;
    let mut failed_count = 0;

    for file_request in request.files {
        let file_start_time = std::time::Instant::now();
        
        match processor.process_file(&file_request.content, false, &file_request.filename, &file_request.source_id, "unknown/repo", &file_request.user_id).await {
            Ok(data) => {
                let processing_time = file_start_time.elapsed().as_millis() as u64;
                results.push(AnalyzeCodeResponse {
                    success: true,
                    data: Some(data),
                    error: None,
                    processing_time_ms: processing_time,
                });
                analyzed_count += 1;
            }
            Err(e) => {
                let processing_time = file_start_time.elapsed().as_millis() as u64;
                results.push(AnalyzeCodeResponse {
                    success: false,
                    data: None,
                    error: Some(e.to_string()),
                    processing_time_ms: processing_time,
                });
                failed_count += 1;
            }
        }
    }

    let total_processing_time = start_time.elapsed().as_millis() as u64;

    Ok(Json(BatchAnalyzeResponse {
        success: failed_count == 0,
        analyzed_files: analyzed_count,
        failed_files: failed_count,
        results,
        total_processing_time_ms: total_processing_time,
    }))
}

#[derive(Debug, Serialize)]
pub struct SupportedLanguagesResponse {
    pub languages: Vec<String>,
}

pub async fn list_supported_languages(
    State(_processor): State<Arc<UnifiedProcessor>>,
) -> Result<Json<SupportedLanguagesResponse>, StatusCode> {
    // This would need to be implemented in the UnifiedProcessor
    // For now, return a static list
    Ok(Json(SupportedLanguagesResponse {
        languages: vec![
            "rust".to_string(),
            "python".to_string(),
            "javascript".to_string(),
            "typescript".to_string(),
            "go".to_string(),
            "java".to_string(),
            "c".to_string(),
            "cpp".to_string(),
        ],
    }))
}

#[derive(Debug, Deserialize)]
pub struct CodeMetricsRequest {
    pub content: String,
    pub filename: String,
}

#[derive(Debug, Serialize)]
pub struct CodeMetricsResponse {
    pub success: bool,
    pub metrics: Option<crate::processors::codebase::CodeMetrics>,
    pub error: Option<String>,
}

pub async fn get_code_metrics(
    State(processor): State<Arc<UnifiedProcessor>>,
    Json(request): Json<CodeMetricsRequest>,
) -> Result<Json<CodeMetricsResponse>, StatusCode> {
    match processor.process_file(&request.content, false, &request.filename, "metrics-request", "unknown/repo", "system").await {
        Ok(data) => {
            let crate::core::orchestrator::ContentType::Code(code_data) = data.content_type;
            Ok(Json(CodeMetricsResponse {
                success: true,
                metrics: Some(crate::processors::codebase::CodeMetrics {
                    lines_of_code: code_data.metrics.lines_of_code,
                    lines_of_comments: code_data.metrics.lines_of_comments,
                    cyclomatic_complexity: code_data.metrics.cyclomatic_complexity,
                    cognitive_complexity: code_data.metrics.cognitive_complexity,
                    maintainability_index: code_data.metrics.maintainability_index,
                }),
                error: None,
            }))
        }
        Err(e) => Ok(Json(CodeMetricsResponse {
            success: false,
            metrics: None,
            error: Some(e.to_string()),
        })),
    }
}

// (Documents removed)


#[derive(Debug, Deserialize)]
pub struct TriggerGraphSyncRequest {
    pub source_id: String,
    pub force_rebuild: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct TriggerGraphSyncResponse {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

pub async fn trigger_graph_sync(
    headers: axum::http::HeaderMap,
    State(processor): State<Arc<UnifiedProcessor>>,
    Json(request): Json<TriggerGraphSyncRequest>,
) -> Result<Json<TriggerGraphSyncResponse>, StatusCode> {
    let user_id = headers.get("x-user-id").and_then(|h| h.to_str().ok()).unwrap_or("system");
    match processor.trigger_graph_sync(&request.source_id, user_id).await {
        Ok(_) => Ok(Json(TriggerGraphSyncResponse {
            success: true,
            message: "Graph sync triggered successfully".to_string(),
            error: None,
        })),
        Err(e) => Ok(Json(TriggerGraphSyncResponse {
            success: false,
            message: "Failed to trigger graph sync".to_string(),
            error: Some(e.to_string()),
        })),
    }
}

#[derive(Debug, Serialize)]
pub struct ProcessingStatusResponse {
    pub source_id: String,
    pub total_files: usize,
    pub processed_files: usize,
    pub graph_built: bool,
    pub last_updated: String,
}

pub async fn get_processing_status(
    headers: axum::http::HeaderMap,
    State(processor): State<Arc<UnifiedProcessor>>,
    Path(source_id): Path<String>,
) -> Result<Json<ProcessingStatusResponse>, StatusCode> {
    let user_id = match headers.get("x-user-id").and_then(|h| h.to_str().ok()) {
        Some(uid) => uid,
        None => return Err(StatusCode::UNAUTHORIZED),
    };
    match processor.get_processing_status(&source_id, user_id).await {
        Ok(status) => Ok(Json(ProcessingStatusResponse {
            source_id: status.source_id,
            total_files: status.total_files,
            processed_files: status.processed_files,
            graph_built: status.graph_built,
            last_updated: status.last_updated,
        })),
        Err(_e) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

// ==========================================
// From health.rs
// ==========================================

// Health check endpoints

/// Health check response
#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub service: String,
    pub version: String,
}

/// Detailed status response
#[derive(Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub status: String,
    pub service: String,
    pub version: String,
    pub components: ComponentStatus,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ComponentStatus {
    pub tree_sitter: String,
    pub docling: String,
    pub embedding_model: String,
    pub postgres: String,
}

/// Health check endpoint
pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        service: "unified-processor".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Detailed status endpoint
pub async fn get_status(State(state): State<Arc<AppState>>) -> Json<StatusResponse> {
    let capabilities = state.processor.get_capabilities();

    Json(StatusResponse {
        status: "running".to_string(),
        service: "unified-processor".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        components: ComponentStatus {
            tree_sitter: if capabilities.tree_sitter_enabled {
                "enabled".to_string()
            } else {
                "disabled".to_string()
            },
            docling: if capabilities.docling_enabled {
                "enabled".to_string()
            } else {
                "disabled".to_string()
            },
            embedding_model: if capabilities.docling_enabled { "active".to_string() } else { "inactive".to_string() },
            postgres: "connected".to_string(),
        },
    })
}

