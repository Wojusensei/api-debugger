// ====================
// 导入依赖
// ====================

use axum::{
    extract::{DefaultBodyLimit, Json as AxumJson, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::post,
    Router,
};
use reqwest::{
    header::{HeaderName, HeaderValue},
    Client, Method, Url,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower_http::services::ServeDir;

// ====================
// 常量
// ====================

/// 转发请求的总超时秒数
const REQUEST_TIMEOUT_SECS: u64 = 30;

/// 响应体读取上限，防止大响应把进程内存撑爆
const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

// ====================
// 数据结构定义
// ====================

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ApiRequest {
    method: String,
    url: String,
    // 用 Vec 保留同名头的多次出现（如两个 Cookie），HashMap 会互相覆盖
    headers: Vec<(String, String)>,
    body: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
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
    port: u16,
}

// ====================
// 本机来源校验（防浏览器端跨站 CSRF 与 DNS rebinding）
// ====================

fn is_local_hostname(host: &str) -> bool {
    // Host 头三种形态："hostname:port"、"[ipv6]:port"、裸 IPv6（::1，多个冒号无方括号）
    let (hostname, port) = if let Some(rest) = host.strip_prefix('[') {
        match rest.split_once(']') {
            Some((hn, after)) => (hn, after.strip_prefix(':')),
            None => return false,
        }
    } else if host.matches(':').count() == 1 {
        let (hn, p) = host.split_once(':').unwrap();
        (hn, Some(p))
    } else {
        (host, None)
    };
    if !matches!(
        hostname.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1"
    ) {
        return false;
    }
    // 冒号后面的必须是合法端口号，别让垃圾段混过去
    port.map_or(true, |p| p.parse::<u16>().is_ok())
}

async fn local_origin_guard(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    req: Request,
    next: Next,
) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !is_local_hostname(host) {
        return (StatusCode::FORBIDDEN, "Host 校验失败：仅允许本机访问").into_response();
    }

    // 浏览器跨站请求必然携带非本机 Origin；curl 等本机工具不携带 Origin，直接放行
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let allowed = [
            format!("http://127.0.0.1:{}", state.port),
            format!("http://localhost:{}", state.port),
            format!("http://[::1]:{}", state.port),
        ];
        if !allowed.iter().any(|a| a == origin) {
            return (StatusCode::FORBIDDEN, "Origin 校验失败：已拦截跨站请求").into_response();
        }
    }

    next.run(req).await
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
    let mut resp = req_builder.send().await.map_err(|e| {
        if e.is_timeout() {
            // 连接超时和总超时都会走到这里，具体秒数各不相同，提示里就不写死了
            ApiError::bad_gateway("请求超时：目标长时间无响应".to_string())
        } else {
            ApiError::bad_gateway(format!("请求目标失败: {e}"))
        }
    })?;

    let status = resp.status().as_u16();
    // Vec 保留同名头的多次出现（如多个 Set-Cookie），HashMap 会互相覆盖
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    // 分块读取响应体并限制大小，text() 会无条件读完全部内容，大文件会把内存撑爆
    let mut raw: Vec<u8> = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| ApiError::bad_gateway(format!("读取响应体失败: {e}")))?
    {
        if raw.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(ApiError::bad_gateway(format!(
                "响应体超过 {} MB 上限，已中断读取",
                MAX_BODY_BYTES / 1024 / 1024
            )));
        }
        raw.extend_from_slice(&chunk);
    }
    // 非 UTF-8 内容按有损方式转字符串（与原 text() 的默认行为一致）
    let body = String::from_utf8_lossy(&raw).into_owned();
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

/// 打日志用的 URL：抹掉用户名密码，凭据不该出现在终端记录里
fn redact_url(url: &str) -> String {
    match Url::parse(url) {
        Ok(mut u) => {
            if !u.username().is_empty() || u.password().is_some() {
                let _ = u.set_username("");
                let _ = u.set_password(None);
            }
            u.to_string()
        }
        Err(_) => url.to_string(),
    }
}

async fn send_request_post(
    State(state): State<Arc<AppState>>,
    AxumJson(req): AxumJson<ApiRequest>,
) -> Result<Json<ApiResponse>, ApiError> {
    let result = execute_request(&state.client, &req).await;
    match &result {
        Ok(resp) => println!(
            "[✓] {} {} · {} · {}ms",
            req.method.to_uppercase(),
            redact_url(&req.url),
            resp.status,
            resp.elapsed_ms
        ),
        Err(e) => println!(
            "[✗] {} {} · {} {}",
            req.method.to_uppercase(),
            redact_url(&req.url),
            e.status,
            e.message
        ),
    }
    result.map(Json)
}

// ====================
// 启动入口
// ====================

#[tokio::main]
async fn main() {
    let client = Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .connect_timeout(Duration::from_secs(10))
        .user_agent("api-debugger/0.1")
        .build()
        .expect("构建 HTTP 客户端失败");

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(5000);
    let state = Arc::new(AppState { client, port });

    let app = build_router(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("[*] API 调试器已启动: http://{}", addr);

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
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

/// 路由与中间件的完整装配，main 和测试共用同一份
fn build_router(state: Arc<AppState>) -> Router {
    let static_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("static");
    Router::new()
        .route("/api/send", post(send_request_post))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            local_origin_guard,
        ))
        .fallback_service(ServeDir::new(static_dir))
        // 请求体上限与响应体上限保持一致（axum 默认只有 2MB）
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    println!("\n[*] 收到 Ctrl+C，等待进行中的请求处理完…");
}

// ====================
// 测试
// ====================

#[cfg(test)]
mod unit_tests {
    use super::*;

    fn api_req(
        method: &str,
        url: &str,
        headers: Vec<(String, String)>,
        body: Option<String>,
    ) -> ApiRequest {
        ApiRequest {
            method: method.into(),
            url: url.into(),
            headers,
            body,
        }
    }

    #[test]
    fn local_hostname_accepts_loopback_forms() {
        for host in [
            "127.0.0.1:5000",
            "localhost:5000",
            "LOCALHOST:5000",
            "LocalHost:80",
            "[::1]:5000",
            "127.0.0.1",
            "::1",
            "localhost:05000",
        ] {
            assert!(is_local_hostname(host), "`{host}` 应视为本机");
        }
    }

    #[test]
    fn local_hostname_rejects_spoof_and_remote() {
        for host in [
            "",
            "evil.com",
            "localhost.evil.com",
            "127.0.0.1.evil.com",
            // 第一个冒号后面必须是合法端口，垃圾段不允许混过校验
            "localhost:5000.evil.com",
            "localhost:99999",
            "example.com:5000",
            "0.0.0.0:5000",
            "127.0.0.2:5000",
        ] {
            assert!(!is_local_hostname(host), "`{host}` 不应视为本机");
        }
    }

    #[test]
    fn redact_url_strips_credentials() {
        assert_eq!(
            redact_url("http://alice:s3cret@example.com/x"),
            "http://example.com/x"
        );
        assert_eq!(redact_url("http://alice@example.com/"), "http://example.com/");
        assert_eq!(
            redact_url("http://bob:pw@127.0.0.1:9099/echo"),
            "http://127.0.0.1:9099/echo"
        );
        // 没有凭据时原样保留
        assert_eq!(
            redact_url("https://example.com/a?b=1"),
            "https://example.com/a?b=1"
        );
        // 解析失败时原样返回，不 panic
        assert_eq!(redact_url("not a url"), "not a url");
    }

    #[tokio::test]
    async fn invalid_url_maps_to_400() {
        let err = execute_request(&Client::new(), &api_req("GET", "not a url", vec![], None))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("URL 无效"), "{}", err.message);
    }

    #[tokio::test]
    async fn non_http_scheme_maps_to_400() {
        for url in ["file:///etc/passwd", "ftp://example.com/x"] {
            let err = execute_request(&Client::new(), &api_req("GET", url, vec![], None))
                .await
                .unwrap_err();
            assert_eq!(err.status, StatusCode::BAD_REQUEST, "{url}");
            assert!(err.message.contains("协议"), "{}", err.message);
        }
    }

    #[tokio::test]
    async fn crlf_in_header_value_maps_to_400() {
        // 头注入是最经典的攻击面，必须在这里被挡住
        let headers = vec![("x-evil".into(), "a\r\nX-Injected: 1".into())];
        let err = execute_request(
            &Client::new(),
            &api_req("GET", "http://127.0.0.1:9/", headers, None),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("x-evil"), "{}", err.message);
    }

    #[tokio::test]
    async fn invalid_header_name_maps_to_400() {
        let headers = vec![("bad name".into(), "v".into())];
        let err = execute_request(
            &Client::new(),
            &api_req("GET", "http://127.0.0.1:9/", headers, None),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn invalid_method_token_maps_to_400() {
        let err = execute_request(
            &Client::new(),
            &api_req("BAD METHOD", "http://127.0.0.1:9/", vec![], None),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("无效的 HTTP 方法"), "{}", err.message);
    }
}
