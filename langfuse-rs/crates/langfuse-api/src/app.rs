use axum::{middleware, Router};
use sqlx::PgPool;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::decompression::RequestDecompressionLayer;
use tower_http::trace::TraceLayer;
use crate::middleware::{auth, api_key};
use crate::openapi::build_openapi_spec;
use crate::routes;
use crate::public_api;

/// 应用状态，注入到所有 handler
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub jwt_secret: Arc<Vec<u8>>,
    pub worker: Option<Arc<langfuse_ingestion::IngestionProcessor>>,
    /// Failed-sign-in tracker for `POST /api/auth/login`. Shared across
    /// requests, so it lives here rather than in a handler-local static.
    pub login_throttle: Arc<crate::throttle::LoginThrottle>,
}

/// 创建 axum Router
pub fn create_app(state: AppState) -> Router {
    // Auth 路由 (登录/注册 不需要认证)
    let auth_routes = Router::new()
        .nest("/api/auth", routes::auth::router());

    // 需要 JWT session 认证的路由
    let private_routes = Router::new()
        .nest("/api/traces", routes::traces::router())
        .nest("/api/observations", routes::observations::router())
        .nest("/api/scores", routes::scores::router())
        .nest("/api/sessions", routes::sessions::router())
        .nest("/api/datasets", routes::datasets::router())
        .nest("/api/prompts", routes::prompts::router())
        .nest("/api/models", routes::models::router())
        .nest("/api/evals", routes::evals::router())
        .nest("/api/projects", routes::projects::router())
        .nest("/api/organizations", routes::organizations::router())
        .nest("/api/dashboards", routes::dashboards::router())
        .nest("/api/comments", routes::comments::router())
        .nest("/api/media", routes::media::router())
        .nest("/api/automations", routes::automations::router())
        .nest("/api/monitors", routes::monitors::router())
        .nest("/api/users", routes::users::router())
        .nest("/api/api-keys", routes::api_keys::router())
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::verify_session,
        ));

    // 公开路由 (/api/public/*) — ingestion / otel / 读接口需要 API Key
    //
    // `RequestDecompressionLayer` transparently un-gzips `Content-Encoding: gzip`
    // request bodies. Every OTLP/HTTP exporter gzips by default (the Go
    // `otlptracehttp` client LexQA uses does), so the OTLP handler would
    // otherwise receive compressed protobuf.
    let public_routes = Router::new()
        .nest("/api/public", public_api::router())
        .layer(RequestDecompressionLayer::new())
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            api_key::verify_api_key_middleware,
        ));

    // Public health check (不需要认证)
    //
    // Sits outside the API-key layer on purpose: container HEALTHCHECK and
    // orchestrator probes have no credentials to send, and a 401 here would
    // make every container look unhealthy.
    let public_health_router = Router::new()
        .route("/api/public/health", axum::routing::get(public_api::health));

    // Health check (不需要认证)
    let health_router = Router::new()
        .route("/health", axum::routing::get(health_check));

    // OpenAPI JSON endpoint
    let openapi_router = Router::new()
        .route("/api/openapi.json", axum::routing::get(serve_openapi));

    Router::new()
        .merge(health_router)
        .merge(public_health_router)
        .merge(openapi_router)
        .merge(auth_routes)
        .merge(public_routes)
        .merge(private_routes)
        // 全局中间件
        .layer(TraceLayer::new_for_http())
        .layer(cors_layer())
        .with_state(state)
}

/// Cross-origin policy for the private API.
///
/// Defaults to **no cross-origin access at all**, which is what the shipped
/// topology needs: the browser only ever talks to the Next.js layer on its own
/// origin, and Next.js proxies `/api/*` to this process server-side. LexQA
/// calls the public API server-to-server, which CORS does not govern.
///
/// The previous `CorsLayer::permissive()` answered `Access-Control-Allow-Origin:
/// *` on every route. Once the console authenticates with a cookie that is a
/// cross-site request forgery footgun rather than a neutral default, so a
/// deployment that genuinely needs cross-origin access opts in explicitly:
///
/// ```text
/// LANGFUSE_CORS_ALLOWED_ORIGINS="https://langfuse.example.com,http://localhost:3001"
/// ```
fn cors_layer() -> CorsLayer {
    let origins = std::env::var("LANGFUSE_CORS_ALLOWED_ORIGINS").unwrap_or_default();

    let allowed: Vec<axum::http::HeaderValue> = origins
        .split(',')
        .map(str::trim)
        .filter(|o| !o.is_empty())
        .filter_map(|o| match o.parse::<axum::http::HeaderValue>() {
            Ok(v) => Some(v),
            Err(_) => {
                tracing::warn!(origin = %o, "ignoring unparseable origin in LANGFUSE_CORS_ALLOWED_ORIGINS");
                None
            }
        })
        .collect();

    if allowed.is_empty() {
        tracing::debug!("no LANGFUSE_CORS_ALLOWED_ORIGINS set; cross-origin requests are refused");
        // `CorsLayer::new()` sends no `Access-Control-Allow-*` headers, so the
        // browser blocks cross-origin reads. Same-origin requests are unaffected.
        return CorsLayer::new();
    }

    tracing::info!(origins = ?origins, "allowing cross-origin requests from configured origins");
    CorsLayer::new()
        .allow_origin(allowed)
        // Required for the session cookie to travel on a cross-origin call.
        // Safe precisely because the origins above are an explicit allow-list —
        // wildcard origin plus credentials is the combination browsers forbid.
        .allow_credentials(true)
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
}

/// 健康检查
async fn health_check(axum::extract::State(_state): axum::extract::State<AppState>) -> &'static str {
    "OK"
}

/// 生成 OpenAPI JSON (无需认证)
async fn serve_openapi() -> axum::response::Json<serde_json::Value> {
    axum::response::Json(build_openapi_spec())
}

/// 启动 API server（支持优雅关闭）
///
/// `shutdown_signal` is a future that, when resolved, initiates graceful shutdown.
/// The server will stop accepting new connections and drain existing ones.
pub async fn serve(
    state: AppState,
    port: u16,
    hostname: &str,
    shutdown_signal: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let app = create_app(state);

    let addr = format!("{}:{}", hostname, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening: http://{}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await?;
    tracing::info!("API server shut down gracefully");
    Ok(())
}
