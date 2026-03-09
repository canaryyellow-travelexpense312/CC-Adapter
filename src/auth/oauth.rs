use anyhow::{Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::Rng;
use sha2::{Digest, Sha256};

use super::token_store::TokenData;

// ─── OAuth 常數（與 OpenAI Codex CLI 一致） ───
// ─── OAuth constants (matching OpenAI Codex CLI) ───

pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
pub const SCOPE: &str = "openid profile email offline_access";

/// PKCE 驗證碼對
/// PKCE verifier/challenge pair
pub struct PkcePair {
    pub verifier: String,
    pub challenge: String,
}

/// 產生 PKCE 驗證碼與挑戰碼（S256 方法）
/// Generate PKCE verifier and challenge (S256 method)
pub fn generate_pkce() -> PkcePair {
    let mut rng = rand::rng();
    let verifier_bytes: Vec<u8> = (0..32).map(|_| rng.random::<u8>()).collect();
    let verifier = URL_SAFE_NO_PAD.encode(&verifier_bytes);

    let hash = Sha256::digest(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hash);

    PkcePair {
        verifier,
        challenge,
    }
}

/// 產生隨機 state 字串（防 CSRF）
/// Generate a random state string (CSRF protection)
pub fn generate_state() -> String {
    let mut rng = rand::rng();
    let bytes: Vec<u8> = (0..16).map(|_| rng.random::<u8>()).collect();
    hex::encode(&bytes)
}

/// 建構完整的 OAuth 授權 URL
/// Build the full OAuth authorization URL
pub fn build_authorize_url(challenge: &str, state: &str) -> String {
    format!(
        "{authorize}?\
         response_type=code\
         &client_id={client_id}\
         &redirect_uri={redirect_uri}\
         &scope={scope}\
         &code_challenge={challenge}\
         &code_challenge_method=S256\
         &state={state}\
         &id_token_add_organizations=true\
         &codex_cli_simplified_flow=true\
         &originator=codex_cli_rs",
        authorize = AUTHORIZE_URL,
        client_id = CLIENT_ID,
        redirect_uri = urlencoding::encode(REDIRECT_URI),
        scope = urlencoding::encode(SCOPE),
        challenge = challenge,
        state = state,
    )
}

/// 用授權碼交換 access token 和 refresh token
/// Exchange an authorization code for access and refresh tokens
pub async fn exchange_code(code: &str, verifier: &str) -> Result<TokenData> {
    let client = reqwest::Client::new();
    let resp = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form_encode(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", REDIRECT_URI),
        ]))
        .send()
        .await
        .context("無法傳送 token 交換請求 / Failed to send token exchange request")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "Token 交換失敗 HTTP {} / Token exchange failed HTTP {}: {}",
            status,
            status,
            body
        );
    }

    parse_token_response(resp).await
}

/// 使用 refresh token 刷新 access token
/// Refresh the access token using a refresh token
pub async fn refresh_token(refresh_token: &str) -> Result<TokenData> {
    let client = reqwest::Client::new();
    let resp = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form_encode(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
        ]))
        .send()
        .await
        .context("無法傳送 token 刷新請求 / Failed to send token refresh request")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "Token 刷新失敗 HTTP {} / Token refresh failed HTTP {}: {}",
            status,
            status,
            body
        );
    }

    parse_token_response(resp).await
}

/// 解析 token 端點回應
/// Parse the token endpoint response
async fn parse_token_response(resp: reqwest::Response) -> Result<TokenData> {
    let json: serde_json::Value = resp
        .json()
        .await
        .context("無法解析 token 回應 / Failed to parse token response")?;

    let access_token = json["access_token"]
        .as_str()
        .context("回應中缺少 access_token / Missing access_token in response")?
        .to_string();
    let refresh_token = json["refresh_token"]
        .as_str()
        .context("回應中缺少 refresh_token / Missing refresh_token in response")?
        .to_string();
    let expires_in = json["expires_in"]
        .as_u64()
        .context("回應中缺少 expires_in / Missing expires_in in response")?;

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    Ok(TokenData {
        access_token,
        refresh_token,
        expires_at: now_ms + expires_in * 1000,
    })
}

/// 解碼 JWT payload 以提取 claims（不驗證簽名）
/// Decode JWT payload to extract claims (no signature verification)
pub fn decode_jwt_payload(token: &str) -> Result<serde_json::Value> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        anyhow::bail!("無效的 JWT 格式 / Invalid JWT format");
    }

    let payload_b64 = parts[1];
    let padding_len = (4 - payload_b64.len() % 4) % 4;
    let padded = format!("{}{}", payload_b64, "=".repeat(padding_len));

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(padded.as_bytes())
        .or_else(|_| URL_SAFE_NO_PAD.decode(payload_b64.as_bytes()))
        .context("無法解碼 JWT payload / Failed to decode JWT payload")?;

    let payload: serde_json::Value = serde_json::from_slice(&decoded)
        .context("無法解析 JWT payload JSON / Failed to parse JWT payload JSON")?;

    Ok(payload)
}

/// 從 JWT 中提取 ChatGPT account ID
/// Extract ChatGPT account ID from JWT
pub fn extract_account_id(token: &str) -> Result<String> {
    let payload = decode_jwt_payload(token)?;
    let account_id = payload["https://api.openai.com/auth"]["chatgpt_account_id"]
        .as_str()
        .context(
            "JWT 中找不到 chatgpt_account_id / chatgpt_account_id not found in JWT",
        )?
        .to_string();
    Ok(account_id)
}

/// hex 編碼工具（避免引入額外 crate）
/// Hex encoding utility (avoids pulling in an extra crate)
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

/// 將鍵值對編碼為 application/x-www-form-urlencoded 格式
/// Encode key-value pairs into application/x-www-form-urlencoded format
fn form_encode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// URL 編碼工具
/// URL encoding utility
mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut result = String::with_capacity(s.len() * 3);
        for byte in s.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    result.push(byte as char);
                }
                _ => {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
        result
    }
}
