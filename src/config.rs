use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::Deserialize;

#[derive(Parser, Debug)]
#[command(name = "claude-adapter")]
#[command(about = "API 轉接器，讓 Claude Code 能使用 OpenAI/Grok/ChatGPT 供應商 / API adapter that lets Claude Code use OpenAI/Grok/ChatGPT providers")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// 覆寫日誌等級 (trace, debug, info, warn, error)
    /// Override log level (trace, debug, info, warn, error)
    #[arg(long, global = true)]
    pub log_level: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// 啟動 Adapter 代理伺服器（預設命令）
    /// Start the Adapter proxy server (default command)
    Serve(ServeArgs),

    /// 執行 ChatGPT OAuth 登入流程
    /// Run the ChatGPT OAuth login flow
    Login,

    /// 清除已儲存的 OAuth token
    /// Clear saved OAuth tokens
    Logout,
}

#[derive(Parser, Debug, Clone)]
pub struct ServeArgs {
    /// 配置檔路徑
    /// Path to config file
    #[arg(short, long, default_value = "config.toml")]
    pub config: PathBuf,

    /// 覆寫監聽主機
    /// Override listen host
    #[arg(long)]
    pub host: Option<String>,

    /// 覆寫監聽埠號
    /// Override listen port
    #[arg(short, long)]
    pub port: Option<u16>,

    /// 覆寫供應商 API 金鑰
    /// Override provider API key
    #[arg(long)]
    pub api_key: Option<String>,

    /// 覆寫供應商 Base URL
    /// Override provider base URL
    #[arg(long)]
    pub base_url: Option<String>,

    /// 覆寫預設模型
    /// Override default model
    #[arg(long)]
    pub model: Option<String>,
}

impl Default for ServeArgs {
    fn default() -> Self {
        Self {
            config: PathBuf::from("config.toml"),
            host: None,
            port: None,
            api_key: None,
            base_url: None,
            model: None,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub provider: ProviderConfig,
    pub models: ModelsConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    /// 日誌等級（trace, debug, info, warn, error）
    /// Log level (trace, debug, info, warn, error)
    /// 透過 peek_log_level() 在 tracing 初始化前讀取，不直接存取此欄位
    /// Read via peek_log_level() before tracing init; not accessed via this field directly
    #[serde(default = "default_log_level")]
    #[allow(dead_code)]
    pub log_level: String,
    /// 日誌檔案路徑（選填）：設定後日誌會同時寫入此檔案
    /// Log file path (optional): when set, logs are also written to this file
    /// 透過 peek_log_file() 在 tracing 初始化前讀取
    /// Read via peek_log_file() before tracing init
    #[serde(default)]
    #[allow(dead_code)]
    pub log_file: Option<String>,
}

fn default_log_level() -> String {
    "info".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProviderConfig {
    #[serde(rename = "type")]
    pub provider_type: String,
    pub api_key: String,
    pub base_url: String,
    /// 供應商是否支援串流 API（預設 false，Adapter 會模擬 SSE 串流）
    /// Whether the provider supports streaming API (default false, Adapter simulates SSE)
    /// 未來支援真正的串流轉發時會使用此欄位
    /// This field will be used when real streaming passthrough is implemented
    #[serde(default)]
    #[allow(dead_code)]
    pub supports_streaming: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModelsConfig {
    pub default: String,
    #[serde(default)]
    pub mapping: HashMap<String, String>,
}

impl Config {
    /// 從配置檔中快速讀取 log_level（不需要完整驗證 config）
    /// Quickly read log_level from config file (without full config validation)
    pub fn peek_log_level(config_path: &std::path::Path) -> Option<String> {
        let content = std::fs::read_to_string(config_path).ok()?;
        let table: toml::Table = toml::from_str(&content).ok()?;
        table
            .get("server")?
            .get("log_level")?
            .as_str()
            .map(|s| s.to_string())
    }

    /// 從配置檔中快速讀取 log_file（不需要完整驗證 config）
    /// Quickly read log_file from config file (without full config validation)
    pub fn peek_log_file(config_path: &std::path::Path) -> Option<String> {
        let content = std::fs::read_to_string(config_path).ok()?;
        let table: toml::Table = toml::from_str(&content).ok()?;
        table
            .get("server")?
            .get("log_file")?
            .as_str()
            .map(|s| s.to_string())
    }

    /// 載入配置：依序從配置檔、環境變數、CLI 參數合併設定
    /// Load config: merge settings from config file, environment variables, and CLI arguments
    pub fn load(args: &ServeArgs) -> Result<Self> {
        let config_path = &args.config;

        let mut config = if config_path.exists() {
            let content = std::fs::read_to_string(config_path)
                .with_context(|| format!("無法讀取配置檔 / Failed to read config file: {}", config_path.display()))?;
            toml::from_str::<Config>(&content)
                .with_context(|| format!("無法解析配置檔 / Failed to parse config file: {}", config_path.display()))?
        } else {
            // 配置檔不存在時使用預設值
            // Use defaults when config file does not exist
            Config {
                server: ServerConfig {
                    host: "127.0.0.1".to_string(),
                    port: 8080,
                    log_level: "info".to_string(),
                    log_file: None,
                },
                provider: ProviderConfig {
                    provider_type: "openai".to_string(),
                    api_key: String::new(),
                    base_url: "https://api.openai.com/v1".to_string(),
                    supports_streaming: false,
                },
                models: ModelsConfig {
                    default: "gpt-4o".to_string(),
                    mapping: HashMap::new(),
                },
            }
        };

        // CLI 參數覆寫配置檔的值
        // CLI arguments override config file values
        if let Some(host) = &args.host {
            config.server.host = host.clone();
        }
        if let Some(port) = args.port {
            config.server.port = port;
        }
        if let Some(base_url) = &args.base_url {
            config.provider.base_url = base_url.clone();
        }
        if let Some(model) = &args.model {
            config.models.default = model.clone();
        }

        // API 金鑰優先順序：CLI 參數 > 環境變數 > 配置檔
        // API key priority: CLI arg > env var > config file
        if let Some(api_key) = &args.api_key {
            config.provider.api_key = api_key.clone();
        } else if let Ok(env_key) = std::env::var("ADAPTER_API_KEY") {
            config.provider.api_key = env_key;
        }

        // 若 provider 類型為 chatgpt，則不需要 API key（使用 OAuth）
        // If provider type is chatgpt, API key is not required (uses OAuth)
        if config.provider.provider_type != "chatgpt" && config.provider.api_key.is_empty() {
            // 檢查是否有 OAuth token — 若有，自動切換為 chatgpt 供應商
            // Check for existing OAuth tokens — if found, auto-switch to chatgpt provider
            let has_tokens = dirs::home_dir()
                .map(|h| h.join(".claude-adapter").join("tokens.json").exists())
                .unwrap_or(false);

            if has_tokens {
                eprintln!(
                    "\n  ⟳ API key 為空但偵測到 OAuth token，自動切換為 chatgpt 供應商"
                );
                eprintln!(
                    "  ⟳ API key is empty but OAuth token found, auto-switching to chatgpt provider\n"
                );
                config.provider.provider_type = "chatgpt".to_string();
                config.provider.base_url = "https://chatgpt.com/backend-api".to_string();
            } else {
                anyhow::bail!(
                    "未設定 API 金鑰。請透過以下方式設定：\n  \
                     No API key configured. Set it via:\n  \
                     - CLI: --api-key <KEY>\n  \
                     - Env: ADAPTER_API_KEY=<KEY>\n  \
                     - Config: provider.api_key in {}\n\n  \
                     或使用 `claude-adapter login` 登入 ChatGPT OAuth\n  \
                     Or use `claude-adapter login` for ChatGPT OAuth",
                    config_path.display()
                );
            }
        }

        Ok(config)
    }
}
