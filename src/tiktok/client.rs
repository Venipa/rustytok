use once_cell::sync::{Lazy, OnceCell};
use reqwest::{header, Client, RequestBuilder};

use crate::config::Config;
use crate::error::AppError;
use super::parser;
use super::types::{UserInfo, VideoInfo, TagInfo};

static HTTP_CLIENT: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("Failed to create HTTP client")
});

/// Cookie header built from API_VERIFYFP / API_DEVICE_ID (ProxiTok-compatible)
static ANTIBOT_COOKIE: OnceCell<Option<String>> = OnceCell::new();

pub fn configure(config: &Config) {
    let cookie = build_antibot_cookie(config.api_verify_fp.as_deref(), config.api_device_id.as_deref());
    let _ = ANTIBOT_COOKIE.set(cookie);
}

fn build_antibot_cookie(verify_fp: Option<&str>, device_id: Option<&str>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(fp) = verify_fp {
        // ProxiTok / TikScraper: Cookie s_v_web_id = verifyFp
        parts.push(format!("s_v_web_id={}", fp));
    }
    if let Some(id) = device_id {
        parts.push(format!("tt_webid={}", id));
        parts.push(format!("tt_webid_v2={}", id));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

pub fn get_http_client() -> &'static Client {
    &HTTP_CLIENT
}

fn tiktok_get(url: &str) -> RequestBuilder {
    let mut req = HTTP_CLIENT
        .get(url)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Referer", "https://www.tiktok.com/");

    if let Some(Some(cookie)) = ANTIBOT_COOKIE.get() {
        req = req.header(header::COOKIE, cookie);
    }

    req
}

/// Fetch user profile and videos
pub async fn fetch_user(username: &str) -> Result<UserInfo, AppError> {
    let url = format!("https://www.tiktok.com/@{}", username);

    let response = tiktok_get(&url)
        .send()
        .await
        .map_err(|e| AppError::FetchError(e.to_string()))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::NotFound);
    }

    if !response.status().is_success() {
        return Err(AppError::FetchError(format!("Status: {}", response.status())));
    }

    let html = response.text().await.map_err(|e| AppError::FetchError(e.to_string()))?;

    parser::parse_user_page(&html, username)
}

/// Fetch single video via `/@user/video/{id}` (current TikTok web URL)
pub async fn fetch_video(username: &str, video_id: &str) -> Result<VideoInfo, AppError> {
    let username = username.trim_start_matches('@');
    let url = format!("https://www.tiktok.com/@{}/video/{}", username, video_id);

    let response = tiktok_get(&url)
        .send()
        .await
        .map_err(|e| AppError::FetchError(e.to_string()))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::NotFound);
    }

    if !response.status().is_success() {
        return Err(AppError::FetchError(format!("Status: {}", response.status())));
    }

    let html = response.text().await.map_err(|e| AppError::FetchError(e.to_string()))?;

    parser::parse_video_page(&html, video_id)
}

/// Fetch tag/hashtag videos
pub async fn fetch_tag(tag_name: &str) -> Result<TagInfo, AppError> {
    let url = format!("https://www.tiktok.com/tag/{}", tag_name);

    let response = tiktok_get(&url)
        .send()
        .await
        .map_err(|e| AppError::FetchError(e.to_string()))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::NotFound);
    }

    if !response.status().is_success() {
        return Err(AppError::FetchError(format!("Status: {}", response.status())));
    }

    let html = response.text().await.map_err(|e| AppError::FetchError(e.to_string()))?;

    parser::parse_tag_page(&html, tag_name)
}

#[cfg(test)]
mod tests {
    use super::build_antibot_cookie;

    #[test]
    fn cookie_from_verify_fp_and_device_id() {
        let cookie = build_antibot_cookie(Some("verify_abc"), Some("12345")).unwrap();
        assert!(cookie.contains("s_v_web_id=verify_abc"));
        assert!(cookie.contains("tt_webid=12345"));
        assert!(cookie.contains("tt_webid_v2=12345"));
    }

    #[test]
    fn cookie_none_when_empty() {
        assert!(build_antibot_cookie(None, None).is_none());
    }
}
