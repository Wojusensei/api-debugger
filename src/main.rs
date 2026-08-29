// ====================
// 导入依赖
// ====================

use axum::{
    extract::{Json as AxumJson, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use reqwest::{
    header::{HeaderName, HeaderValue},
    Client, Method, Url,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
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
    headers: Vec<(String, String)>,
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
// 错误类型（携带可读信息返回给前端）
// ====================

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }

    fn bad_gateway(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: msg.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

// ====================
// 请求执行（公共逻辑）
// ====================

async fn execute_request(
    client: &Client,
    req: &ApiRequest,
) -> Result<ApiResponse, ApiError> {
    let method = Method::from_bytes(req.method.to_uppercase().as_bytes())
        .map_err(|e| ApiError::bad_request(format!("无效的 HTTP 方法 `{}`: {e}", req.method)))?;

    let url = Url::parse(&req.url)
        .map_err(|e| ApiError::bad_request(format!("URL 无效 `{}`: {e}", req.url)))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ApiError::bad_request(format!(
            "仅支持 http/https 协议，收到 `{}`",
            url.scheme()
        )));
    }

    let mut req_builder = client.request(method, url);

    for (key, value) in &req.headers {
        let name = HeaderName::try_from(key.as_str())
            .map_err(|e| ApiError::bad_request(format!("无效的请求头名称 `{key}`: {e}")))?;
        let value = HeaderValue::from_str(value)
            .map_err(|e| ApiError::bad_request(format!("无效的请求头值 `{key}`: {e}")))?;
        req_builder = req_builder.header(name, value);
    }

    if let Some(body) = &req.body {
        if !body.is_empty() {
            req_builder = req_builder.body(body.clone());
        }
    }

    let start = Instant::now();
    let resp = req_builder
        .send()
        .await
        .map_err(|e| ApiError::bad_gateway(format!("请求目标失败: {e}")))?;

    let status = resp.status().as_u16();
    // Vec 保留同名头的多次出现（如多个 Set-Cookie），HashMap 会互相覆盖
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body = resp
        .text()
        .await
        .map_err(|e| ApiError::bad_gateway(format!("读取响应体失败: {e}")))?;
    // 计时覆盖到响应体读取完成，而不仅是收到响应头
    let elapsed_ms = start.elapsed().as_millis() as u64;

    Ok(ApiResponse {
        status,
        headers,
        body,
        elapsed_ms,
    })
}

// ====================
// POST 请求处理
// ====================

async fn send_request_post(
    State(state): State<Arc<AppState>>,
    AxumJson(req): AxumJson<ApiRequest>,
) -> Result<Json<ApiResponse>, ApiError> {
    execute_request(&state.client, &req).await.map(Json)
}

// ====================
// GET 请求处理
// ====================

async fn send_request_get(
    Query(params): Query<HashMap<String, String>>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<ApiResponse>, ApiError> {
    let req = ApiRequest {
        method: params.get("method").cloned().unwrap_or_else(|| "GET".into()),
        url: params.get("url").cloned().unwrap_or_default(),
        headers: HashMap::new(),
        body: None,
    };

    execute_request(&state.client, &req).await.map(Json)
}

// ====================
// 启动入口
// ====================

#[tokio::main]
async fn main() {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .user_agent("api-debugger/0.1")
        .build()
        .expect("构建 HTTP 客户端失败");

    let state = Arc::new(AppState { client });

    let static_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("static");

    let app = Router::new()
        .route("/api/send", get(send_request_get).post(send_request_post))
        .fallback_service(ServeDir::new(static_dir))
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(5000);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("[*] API 调试器已启动♿️: http://{}", addr);

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "[!] 监听 {} 失败: {}（macOS 上 5000 端口常被 AirPlay 接收器占用，可用 PORT=5050 cargo run --release 换端口）",
                addr, e
            );
            std::process::exit(1);
        }
    };
    axum::serve(listener, app).await.unwrap();
}
