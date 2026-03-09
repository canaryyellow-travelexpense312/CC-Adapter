use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::{Query, State as AxumState};
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use tokio::sync::oneshot;

/// 回調伺服器收到的授權碼
/// Authorization code received by the callback server
#[derive(Debug)]
pub struct AuthCode {
    pub code: String,
}

/// 回調查詢參數
/// Callback query parameters
#[derive(Debug, serde::Deserialize)]
pub struct CallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
}

struct CallbackState {
    expected_state: String,
    tx: std::sync::Mutex<Option<oneshot::Sender<AuthCode>>>,
}

const SUCCESS_HTML: &str = r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>OAuth 成功 / OAuth Success</title>
<style>body{display:flex;justify-content:center;align-items:center;min-height:100vh;
font-family:system-ui,sans-serif;background:#f0f4f8;margin:0}
.card{background:#fff;padding:3rem;border-radius:16px;box-shadow:0 4px 24px rgba(0,0,0,.08);
text-align:center;max-width:400px}
h1{color:#10b981;margin:0 0 1rem}p{color:#64748b;line-height:1.6}</style>
</head><body><div class="card"><h1>&#10003; 登入成功</h1>
<p>OAuth 認證已完成，您可以關閉此視窗。<br/>
OAuth authentication complete. You may close this window.</p></div></body></html>"#;

/// 啟動本地 OAuth 回調伺服器，等待授權碼
/// Start a local OAuth callback server and wait for the authorization code
///
/// 在 127.0.0.1:1455 啟動臨時 HTTP 伺服器，
/// 處理 GET /auth/callback 並透過 oneshot channel 回傳授權碼。
/// 超時 120 秒後自動關閉。
///
/// Starts a temporary HTTP server on 127.0.0.1:1455,
/// handles GET /auth/callback and returns the code via a oneshot channel.
/// Auto-closes after 120 seconds.
pub async fn wait_for_callback(expected_state: String) -> Result<AuthCode> {
    let (tx, rx) = oneshot::channel::<AuthCode>();

    let state = Arc::new(CallbackState {
        expected_state,
        tx: std::sync::Mutex::new(Some(tx)),
    });

    let app = Router::new()
        .route("/auth/callback", get(handle_callback))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:1455")
        .await
        .context(
            "無法綁定 127.0.0.1:1455，請確認埠號未被佔用 / \
             Failed to bind 127.0.0.1:1455, ensure the port is not in use",
        )?;

    let server = axum::serve(listener, app);

    // 120 秒超時
    // 120-second timeout
    let result = tokio::select! {
        code = rx => {
            code.context("回調 channel 已關閉 / Callback channel closed")
        }
        _ = tokio::time::sleep(std::time::Duration::from_secs(120)) => {
            anyhow::bail!(
                "等待 OAuth 回調超時（120 秒）/ OAuth callback timed out (120 seconds)"
            )
        }
        res = server => {
            res.context("回調伺服器異常終止 / Callback server terminated unexpectedly")?;
            anyhow::bail!("回調伺服器意外結束 / Callback server ended unexpectedly")
        }
    };

    result
}

/// 處理 GET /auth/callback 請求
/// Handle GET /auth/callback request
async fn handle_callback(
    AxumState(state): AxumState<Arc<CallbackState>>,
    Query(params): Query<CallbackParams>,
) -> Result<Html<&'static str>, (axum::http::StatusCode, String)> {
    let code = params.code.ok_or_else(|| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            "缺少授權碼 / Missing authorization code".to_string(),
        )
    })?;

    let received_state = params.state.ok_or_else(|| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            "缺少 state 參數 / Missing state parameter".to_string(),
        )
    })?;

    if received_state != state.expected_state {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "State 不匹配（可能遭受 CSRF 攻擊）/ State mismatch (possible CSRF attack)"
                .to_string(),
        ));
    }

    if let Some(tx) = state.tx.lock().unwrap().take() {
        let _ = tx.send(AuthCode { code });
    }

    Ok(Html(SUCCESS_HTML))
}
