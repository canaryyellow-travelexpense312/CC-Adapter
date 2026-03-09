use serde::{Deserialize, Serialize};
use serde_json::Value;

// ─── Responses API 請求型別 ───
// ─── Responses API Request Types ───

/// OpenAI Responses API 請求（ChatGPT Codex 後端使用）
/// OpenAI Responses API request (used by ChatGPT Codex backend)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesRequest {
    pub model: String,
    pub input: Vec<InputItem>,
    /// 固定為 false（ChatGPT 後端要求）
    /// Always false (ChatGPT backend requirement)
    pub store: bool,
    /// 固定為 true（Codex 後端總是回傳 SSE）
    /// Always true (Codex backend always returns SSE)
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ResponsesTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
}

/// 輸入項目：依 type 欄位區分為訊息、函式呼叫、函式呼叫結果
/// Input item: discriminated by `type` into message, function_call, function_call_output
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum InputItem {
    /// 對話訊息
    /// Conversation message
    #[serde(rename = "message")]
    Message {
        role: String,
        content: InputContent,
    },
    /// 函式（工具）呼叫
    /// Function (tool) call
    #[serde(rename = "function_call")]
    FunctionCall {
        name: String,
        arguments: String,
        call_id: String,
    },
    /// 函式呼叫結果
    /// Function call output
    #[serde(rename = "function_call_output")]
    FunctionCallOutput {
        call_id: String,
        output: String,
    },
}

/// 輸入訊息的內容：可以是純文字或內容片段陣列
/// Input message content: either plain text or an array of content parts
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InputContent {
    Text(String),
    Parts(Vec<InputContentPart>),
}

/// 輸入內容片段
/// Input content part
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum InputContentPart {
    /// 文字片段
    /// Text part
    #[serde(rename = "input_text")]
    Text { text: String },
    /// 圖片片段（base64）
    /// Image part (base64)
    #[serde(rename = "input_image")]
    Image {
        image_url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}

/// Responses API 工具定義
/// Responses API tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsesTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
}

/// 推理配置
/// Reasoning configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// 文字輸出配置
/// Text output configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
}

// ─── Responses API SSE 回應型別 ───
// ─── Responses API SSE Response Types ───

/// SSE 事件資料（Responses API 回傳的串流事件）
/// SSE event data (streaming events returned by Responses API)
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct SseEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(flatten)]
    pub data: Value,
}

/// 完整的 Responses API 回應（從 response.completed 事件提取）
/// Complete Responses API response (extracted from response.completed event)
#[derive(Debug, Clone, Deserialize)]
pub struct ResponsesResponse {
    #[allow(dead_code)]
    pub id: String,
    pub status: String,
    #[allow(dead_code)]
    pub model: String,
    #[serde(default)]
    pub output: Vec<OutputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ResponsesUsage>,
}

/// 輸出項目：訊息、函式呼叫或其他（reasoning 等）
/// Output item: message, function call, or other (reasoning, etc.)
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum OutputItem {
    /// 訊息輸出
    /// Message output
    #[serde(rename = "message")]
    Message {
        #[serde(default)]
        content: Vec<OutputContent>,
        #[allow(dead_code)]
        #[serde(default)]
        role: String,
    },
    /// 函式呼叫輸出
    /// Function call output
    #[serde(rename = "function_call")]
    FunctionCall {
        name: String,
        arguments: String,
        call_id: String,
    },
    /// 未知類型（reasoning、web_search 等），安全忽略
    #[serde(other)]
    Unknown,
}

/// 輸出訊息的內容片段
/// Output message content part
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum OutputContent {
    /// 文字輸出
    /// Text output
    #[serde(rename = "output_text")]
    Text { text: String },
    /// 未知類型（refusal 等），安全忽略
    #[serde(other)]
    Unknown,
}

/// Responses API 使用量統計
/// Responses API usage statistics
#[derive(Debug, Clone, Deserialize)]
pub struct ResponsesUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[allow(dead_code)]
    #[serde(default)]
    pub total_tokens: u32,
}
