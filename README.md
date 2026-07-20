# API调试器

本地优先的超级简易版 API 调试工具
基于 Rust + Axum，轻量、离线、数据本地化

## ✨ 功能

- 发送 GET / POST / PUT / DELETE 请求
- 自定义 Headers 和 Body
- JSON 响应自动格式化，语法高亮
- 请求历史记录
- Web 前端界面
- 数据本地存储

## 📦 快速开始

### 1. 克隆项目

```bash
git clone https://github.com/Wojusensei/api-debugger.git
cd api-debugger
```

### 2.编译运行

```bash
cargo build --release
cargo run --release
```

### 3.打开界面
访问 http://127.0.0.1:5000

### 4.技术栈

Rust - 后端

Axum - Web 框架

Reqwest - HTTP 客户端

Tower-HTTP - 静态文件服务

### 5.项目结构

api-debugger/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── src/
│   └── main.rs          
└── static/
    └── index.html       







