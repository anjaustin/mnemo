use axum::{
    http::{header, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "dashboard/"]
struct DashboardAssets;

/// Serve the main dashboard HTML page for all `/_/` routes.
/// Client-side JS handles routing between pages.
async fn dashboard_index() -> Html<String> {
    match DashboardAssets::get("index.html") {
        Some(content) => {
            let html = String::from_utf8_lossy(content.data.as_ref()).into_owned();
            Html(html)
        }
        None => Html("<!-- dashboard index.html not found -->".to_string()),
    }
}

/// Serve a static asset from the embedded `dashboard/` directory.
async fn dashboard_static(axum::extract::Path(filename): axum::extract::Path<String>) -> Response {
    match DashboardAssets::get(&filename) {
        Some(content) => {
            let mime = mime_guess::from_path(&filename)
                .first_or_octet_stream()
                .to_string();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime)],
                content.data.to_vec(),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// Build the dashboard routes.
///
/// Axum 0.7 forbids wildcard routes inside `.nest()`, so we register
/// each route with its full prefix instead.
///
/// - `/_/`                     → SPA index
/// - `/_/static/{*path}`       → embedded CSS / JS assets
/// - `/_/{*path}`              → SPA catch-all (client-side routing)
pub fn dashboard_routes() -> Router {
    Router::new()
        .route("/_", get(|| async { Redirect::permanent("/_/") }))
        .route("/_/", get(dashboard_index))
        .route("/_/static/*path", get(dashboard_static))
        .route("/_/*path", get(dashboard_spa_catch_all))
}

/// Handle `/_/{path}` — serves the SPA index for any path that doesn't
/// match a more specific route (client-side routing).
async fn dashboard_spa_catch_all(_path: axum::extract::Path<String>) -> Html<String> {
    dashboard_index().await
}
