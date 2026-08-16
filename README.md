# API调试器

本地优先的超级简易版 API 调试工具
基于 Rust + Axum，轻量、离线、本地化

## ✨ 功能

- 发送 GET / POST / PUT / DELETE 请求
- 自定义 Headers 和 Body（标签页切换，可增删行）
- JSON 响应自动格式化，语法高亮
- 响应体 / 响应头标签页查看
- 请求与响应左右分栏布局
- 请求历史记录本地持久化（localStorage），点击回填完整请求
- Cmd/Ctrl + Enter 快捷发送
- Web 前端界面

## 📦 快速开始

### 1. 克隆项目

```bash
git clone https://github.com/Wojusensei/api-debugger.git
cd api-debugger
```

### 2.编译运行

```bash
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







