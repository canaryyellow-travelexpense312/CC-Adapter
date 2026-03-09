use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// OAuth token 資料結構
/// OAuth token data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenData {
    pub access_token: String,
    pub refresh_token: String,
    /// 過期時間（Unix 毫秒）
    /// Expiration time (Unix milliseconds)
    pub expires_at: u64,
}

impl TokenData {
    /// 檢查 token 是否已過期（提前 60 秒判定）
    /// Check whether the token has expired (60-second early buffer)
    pub fn is_expired(&self) -> bool {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        now_ms + 60_000 >= self.expires_at
    }
}

/// 取得 token 儲存目錄：~/.claude-adapter/
/// Get the token storage directory: ~/.claude-adapter/
fn storage_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context(
        "無法取得使用者家目錄 / Failed to get home directory",
    )?;
    Ok(home.join(".claude-adapter"))
}

/// 取得 token 檔案路徑：~/.claude-adapter/tokens.json
/// Get the token file path: ~/.claude-adapter/tokens.json
fn token_path() -> Result<PathBuf> {
    Ok(storage_dir()?.join("tokens.json"))
}

/// 儲存 token 至磁碟
/// Save token to disk
pub fn save(data: &TokenData) -> Result<()> {
    let dir = storage_dir()?;
    if !dir.exists() {
        std::fs::create_dir_all(&dir).with_context(|| {
            format!(
                "無法建立目錄 / Failed to create directory: {}",
                dir.display()
            )
        })?;
    }
    let path = token_path()?;
    let json = serde_json::to_string_pretty(data)
        .context("無法序列化 token / Failed to serialize token")?;
    std::fs::write(&path, json).with_context(|| {
        format!(
            "無法寫入 token 檔案 / Failed to write token file: {}",
            path.display()
        )
    })?;
    Ok(())
}

/// 從磁碟載入 token，不存在則回傳 None
/// Load token from disk; returns None if the file does not exist
pub fn load() -> Result<Option<TokenData>> {
    let path = token_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let json = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "無法讀取 token 檔案 / Failed to read token file: {}",
            path.display()
        )
    })?;
    let data: TokenData = serde_json::from_str(&json)
        .context("無法解析 token 檔案 / Failed to parse token file")?;
    Ok(Some(data))
}

/// 刪除已儲存的 token 檔案
/// Delete the saved token file
pub fn delete() -> Result<()> {
    let path = token_path()?;
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| {
            format!(
                "無法刪除 token 檔案 / Failed to delete token file: {}",
                path.display()
            )
        })?;
    }
    Ok(())
}
