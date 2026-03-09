use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::Sse;
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum::http::StatusCode;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

use crate::config::{Config, ServeArgs};
use crate::convert::request::convert_request;
use crate::convert::response::{convert_response, response_to_sse_events};
use crate::error::AppError;
use crate::providers::openai::OpenAIProvider;
use crate::providers::chatgpt::ChatGPTProvider;
use crate::types::anthropic::MessagesRequest;

/// 供應商列舉：支援 OpenAI（含 Grok）或 ChatGPT OAuth
/// Provider enum: supports OpenAI (including Grok) or ChatGPT OAuth
pub enum ProviderKind {
    OpenAI(OpenAIProvider),
    ChatGPT(ChatGPTProvider),
}

/// 應用程式共享狀態（支援熱重載）
/// Application shared state (supports hot-reload)
pub struct AppState {
    pub config: RwLock<Config>,
    pub provider: RwLock<ProviderKind>,
    pub serve_args: ServeArgs,
}

/// 根據配置建構供應商實例
/// Build a provider instance from configuration
async fn build_provider(config: &Config) -> anyhow::Result<ProviderKind> {
    match config.provider.provider_type.as_str() {
        "chatgpt" => {
            let chatgpt = ChatGPTProvider::new().await?;
            Ok(ProviderKind::ChatGPT(chatgpt))
        }
        _ => {
            let openai = OpenAIProvider::new(
                config.provider.api_key.clone(),
                config.provider.base_url.clone(),
            );
            Ok(ProviderKind::OpenAI(openai))
        }
    }
}

/// 根據配置建構 AppState
/// Build AppState based on configuration
pub async fn build_app_state(config: Config, serve_args: ServeArgs) -> anyhow::Result<Arc<AppState>> {
    let provider = build_provider(&config).await?;

    Ok(Arc::new(AppState {
        config: RwLock::new(config),
        provider: RwLock::new(provider),
        serve_args,
    }))
}

/// 重新載入配置並重建供應商（熱重載核心）
/// Reload config and rebuild provider (hot-reload core)
pub async fn reload_config(state: &Arc<AppState>) -> anyhow::Result<()> {
    let new_config = Config::load(&state.serve_args)?;
    let new_provider = build_provider(&new_config).await?;

    let provider_type = new_config.provider.provider_type.clone();
    let base_url = new_config.provider.base_url.clone();
    let default_model = new_config.models.default.clone();

    // 先更新 provider 再更新 config，確保任何新進請求都能使用新的供應商
    // Update provider first, then config, so any new request uses the new provider
    *state.provider.write().await = new_provider;
    *state.config.write().await = new_config;

    eprintln!("\n  ⟳ config.toml 已更新，配置已熱重載");
    eprintln!("  ⟳ config.toml updated, configuration hot-reloaded");
    eprintln!("    供應商 / Provider: {} ({})", provider_type, base_url);
    eprintln!("    預設模型 / Default model: {}\n", default_model);

    info!(
        provider = %provider_type,
        base_url = %base_url,
        default_model = %default_model,
        "配置已熱重載 / Configuration hot-reloaded"
    );

    Ok(())
}

/// 處理 POST /v1/messages 請求：接收 Anthropic 格式、轉換、轉發、回傳
/// 當客戶端請求串流但供應商不支援時，Adapter 會以非串流方式呼叫供應商，再模擬 SSE 事件回傳。
///
/// Handle POST /v1/messages: receive Anthropic format, convert, forward, and return.
/// When client requests streaming but provider doesn't support it, Adapter calls provider
/// without streaming, then simulates SSE events back to the client.
pub async fn handle_messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<MessagesRequest>,
) -> Result<Response, AppError> {
    // 記錄收到的請求資訊
    // Log incoming request details
    let anthropic_version = headers
        .get("anthropic-version")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    let wants_stream = request.stream.unwrap_or(false);
    info!(
        model = %request.model,
        anthropic_version = %anthropic_version,
        max_tokens = request.max_tokens,
        message_count = request.messages.len(),
        has_tools = request.tools.is_some(),
        stream = wants_stream,
        "收到 Anthropic Messages API 請求 / Received Anthropic Messages API request"
    );

    // debug 等級時印出完整的 Anthropic 請求 JSON
    // Print the full Anthropic request JSON at debug level
    if let Ok(json) = serde_json::to_string_pretty(&request) {
        debug!("Anthropic 請求內容 / Anthropic request body:\n{}", json);
    }

    let original_model = request.model.clone();

    // 取得 config 和 provider 的讀取鎖（多個請求可同時讀取，不阻塞）
    // Acquire read locks on config and provider (multiple requests can read concurrently)
    let config = state.config.read().await;
    let provider = state.provider.read().await;

    // 依供應商類型分派不同的轉換與呼叫邏輯
    // Dispatch to different conversion and call logic based on provider type
    let anthropic_response = match &*provider {
        ProviderKind::OpenAI(p) => {
            handle_openai_request(p, request, &config, &original_model).await?
        }
        ProviderKind::ChatGPT(p) => {
            handle_chatgpt_request(p, request, &config, &original_model).await?
        }
    };

    // 釋放鎖（提早 drop 以減少鎖持有時間）
    // Release locks early to minimize hold time
    drop(config);
    drop(provider);

    info!(
        stop_reason = ?anthropic_response.stop_reason,
        content_blocks = anthropic_response.content.len(),
        input_tokens = anthropic_response.usage.input_tokens,
        output_tokens = anthropic_response.usage.output_tokens,
        response_mode = if wants_stream { "SSE 串流 / SSE stream" } else { "JSON" },
        "回傳 Anthropic 回應 / Returning Anthropic response"
    );

    // debug 等級時印出完整的 Anthropic 回應 JSON
    // Print the full Anthropic response JSON at debug level
    if let Ok(json) = serde_json::to_string_pretty(&anthropic_response) {
        debug!("Anthropic 回應內容 / Anthropic response body:\n{}", json);
    }

    // 根據客戶端是否要求串流，決定回傳 JSON 或模擬 SSE 串流
    // Return JSON or simulated SSE stream depending on whether client requested streaming
    if wants_stream {
        let events = response_to_sse_events(&anthropic_response).map_err(|e| {
            error!(error = %e, "SSE 事件轉換失敗 / Failed to convert to SSE events");
            AppError::internal(format!("SSE conversion failed: {}", e))
        })?;

        debug!(
            event_count = events.len(),
            "模擬 SSE 串流回傳 / Simulating SSE stream response"
        );

        let stream = tokio_stream::iter(events.into_iter().map(Ok::<_, Infallible>));
        Ok(Sse::new(stream).into_response())
    } else {
        Ok(Json(anthropic_response).into_response())
    }
}

/// 處理 OpenAI / Grok 供應商的請求轉換與轉發
/// Handle request conversion and forwarding for OpenAI / Grok provider
async fn handle_openai_request(
    provider: &OpenAIProvider,
    request: MessagesRequest,
    config: &Config,
    original_model: &str,
) -> Result<crate::types::anthropic::MessagesResponse, AppError> {
    let openai_request = convert_request(
        request,
        &config.models.mapping,
        &config.models.default,
    )
    .map_err(|e| {
        error!(error = %e, "請求轉換失敗 / Failed to convert request");
        AppError::bad_request(format!("Request conversion failed: {}", e))
    })?;

    debug!(
        openai_model = %openai_request.model,
        openai_messages = openai_request.messages.len(),
        "已轉換為 OpenAI 格式 / Converted to OpenAI format"
    );

    // debug 等級時印出完整的 OpenAI 請求 JSON
    // Print the full OpenAI request JSON at debug level
    if let Ok(json) = serde_json::to_string_pretty(&openai_request) {
        debug!("OpenAI 請求內容 / OpenAI request body:\n{}", json);
    }

    let openai_response = provider
        .chat_completion(openai_request)
        .await
        .map_err(|e| {
            error!(error = %e, "供應商請求失敗 / Provider request failed");
            AppError::internal(format!("Provider error: {}", e))
        })?;

    convert_response(openai_response, original_model).map_err(|e| {
        error!(error = %e, "回應轉換失敗 / Failed to convert response");
        AppError::internal(format!("Response conversion failed: {}", e))
    })
}

/// 處理 ChatGPT 供應商的請求轉換與轉發
/// Handle request conversion and forwarding for ChatGPT provider
async fn handle_chatgpt_request(
    provider: &ChatGPTProvider,
    request: MessagesRequest,
    config: &Config,
    original_model: &str,
) -> Result<crate::types::anthropic::MessagesResponse, AppError> {
    use crate::convert::request_responses::convert_request_to_responses;
    use crate::convert::response_responses::convert_responses_to_anthropic;

    let responses_request = convert_request_to_responses(
        request,
        &config.models.mapping,
        &config.models.default,
    )
    .map_err(|e| {
        error!(error = %e, "Responses API 請求轉換失敗 / Responses API request conversion failed");
        AppError::bad_request(format!("Responses API conversion failed: {}", e))
    })?;

    debug!(
        model = %responses_request.model,
        input_count = responses_request.input.len(),
        "已轉換為 Responses API 格式 / Converted to Responses API format"
    );

    // debug 等級時印出完整的 Responses API 請求 JSON
    // Print the full Responses API request JSON at debug level
    if let Ok(json) = serde_json::to_string_pretty(&responses_request) {
        debug!("Responses API 請求內容 / Responses API request body:\n{}", json);
    }

    let sse_text = provider
        .send_request(&responses_request)
        .await
        .map_err(|e| {
            error!(error = %e, "ChatGPT 供應商請求失敗 / ChatGPT provider request failed");
            AppError::internal(format!("ChatGPT provider error: {}", e))
        })?;

    convert_responses_to_anthropic(&sse_text, original_model).map_err(|e| {
        error!(error = %e, "Responses API 回應轉換失敗 / Responses API response conversion failed");
        AppError::internal(format!("Responses API response conversion failed: {}", e))
    })
}

/// 健康檢查端點
/// Health check endpoint
pub async fn handle_health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}
