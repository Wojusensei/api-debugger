// ====================
// 导入依赖
// ====================

use axum::{
    extract::Json as AxumJson,
    extract::Query,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::services::ServeDir;

// ====================
// 数据结构定义
// ====================

#[derive(Serialize, Deserialize, Clone)]
struct ApiRequest {
    method: String,
    url: String,
    headers: HashMap<String, String>,
    body: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ApiResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
    elapsed_ms: u64,
}

// ====================
// 应用状态
// ====================

struct AppState {
    client: Client,
}

// ====================
// POST 请求处理
// ====================

async fn send_request_post(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    AxumJson(req): AxumJson<ApiRequest>,
) -> Result<Json<ApiResponse>, StatusCode> {
    let method = req.method.to_uppercase();
    let url = req.url;

    let mut req_builder = match method.as_str() {
        "GET" => state.client.get(&url),
        "POST" => state.client.post(&url),
        "PUT" => state.client.put(&url),
        "DELETE" => state.client.delete(&url),
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    for (key, value) in &req.headers {
        req_builder = req_builder.header(key.as_str(), value.as_str());
    }

    if let Some(body) = &req.body {
        req_builder = req_builder.body(body.clone());
    }

    let start = std::time::Instant::now();
    let resp = req_builder.send().await.map_err(|_| StatusCode::BAD_GATEWAY)?;
    let elapsed = start.elapsed().as_millis() as u64;

    let status = resp.status().as_u16();
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body = resp.text().await.unwrap_or_default();

    Ok(Json(ApiResponse {
        status,
        headers,
        body,
        elapsed_ms: elapsed,
    }))
}

// ====================
// GET 请求处理
// ====================

async fn send_request_get(
    Query(params): Query<HashMap<String, String>>,
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Result<Json<ApiResponse>, StatusCode> {
    let method = params.get("method").cloned().unwrap_or("GET".into());
    let url = params.get("url").cloned().unwrap_or_default();

    let req_builder = match method.as_str() {
        "GET" => state.client.get(&url),
        "POST" => state.client.post(&url),
        "PUT" => state.client.put(&url),
        "DELETE" => state.client.delete(&url),
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    let start = std::time::Instant::now();
    let resp = req_builder.send().await.map_err(|_| StatusCode::BAD_GATEWAY)?;
    let elapsed = start.elapsed().as_millis() as u64;

    let status = resp.status().as_u16();
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body = resp.text().await.unwrap_or_default();

    Ok(Json(ApiResponse {
        status,
        headers,
        body,
        elapsed_ms: elapsed,
    }))
}

// ====================
// 启动入口
// ====================

#[tokio::main]
async fn main() {
    let state = Arc::new(AppState {
        client: Client::new(),
    });

    let app = Router::new()
        .route("/api/send", get(send_request_get).post(send_request_post))
        .fallback_service(ServeDir::new("static"))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 5000));
    println!("[*] API 调试器已启动♿️: http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}