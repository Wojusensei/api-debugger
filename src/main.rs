// ====================
// 导入依赖
// ====================

use axum::{
    extract::Query,
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;

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
    history: Mutex<Vec<ApiRequest>>,
}

// ====================
// 请求发送处理
// ====================

async fn send_request(
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
        history: Mutex::new(Vec::new()),
    });

    let app = Router::new()
        .route("/api/send", get(send_request))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 5000));
    println!("[*] API 调试器已启动♿️: http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
