use axum::{
    Json,
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
};
use pinglake_protocol::ApiError;

#[cfg(web_dist_present)]
use axum::{body::Body, http::header};
#[cfg(web_dist_present)]
use rust_embed::RustEmbed;

#[cfg(web_dist_present)]
#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct WebAssets;

pub async fn serve(uri: Uri) -> Response {
    if uri.path().starts_with("/api/") {
        return (
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "API route not found".to_owned(),
            }),
        )
            .into_response();
    }

    serve_asset(uri.path())
}

#[cfg(web_dist_present)]
fn serve_asset(request_path: &str) -> Response {
    let requested = request_path.trim_start_matches('/');
    let requested = if requested.is_empty() || requested.contains("..") {
        "index.html"
    } else {
        requested
    };
    let (path, asset) = WebAssets::get(requested)
        .map(|asset| (requested, asset))
        .or_else(|| WebAssets::get("index.html").map(|asset| ("index.html", asset)))
        .expect("build script only enables embedded web assets when index.html exists");
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache_control = if path == "index.html" {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::CACHE_CONTROL, cache_control)
        .body(Body::from(asset.data))
        .expect("static response contains valid headers")
}

#[cfg(not(web_dist_present))]
fn serve_asset(_request_path: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "PingLake web assets were not present when the Hub was built",
    )
        .into_response()
}
