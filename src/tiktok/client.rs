use std::sync::Arc;

use once_cell::sync::OnceCell;
use reqwest::cookie::Jar;
use reqwest::{Client, RequestBuilder, Url};

use crate::config::Config;
use crate::error::AppError;
use super::parser;
use super::types::{UserInfo, VideoInfo, TagInfo};

static HTTP_CLIENT: OnceCell<Client> = OnceCell::new();

const TIKTOK_ORIGIN: &str = "https://www.tiktok.com";

pub fn configure(config: &Config) {
    let jar = Arc::new(Jar::default());
    let origin: Url = TIKTOK_ORIGIN.parse().expect("valid tiktok origin");

    if let Some(fp) = &config.api_verify_fp {
        jar.add_cookie_str(
            &format!("s_v_web_id={fp}; Domain=.tiktok.com; Path=/; Secure"),
            &origin,
        );
    }
    if let Some(id) = &config.api_device_id {
        jar.add_cookie_str(
            &format!("tt_webid={id}; Domain=.tiktok.com; Path=/"),
            &origin,
        );
        jar.add_cookie_str(
            &format!("tt_webid_v2={id}; Domain=.tiktok.com; Path=/"),
            &origin,
        );
    }
    // Required for v16-webapp-prime playAddr streams (`tk=tt_chain_token`)
    if let Some(token) = &config.api_tt_chain_token {
        jar.add_cookie_str(
            &format!("tt_chain_token={token}; Domain=.tiktok.com; Path=/; Secure"),
            &origin,
        );
    }

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .cookie_provider(jar)
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("Failed to create HTTP client");

    let _ = HTTP_CLIENT.set(client);
}

pub fn get_http_client() -> &'static Client {
    HTTP_CLIENT
        .get()
        .expect("HTTP client used before configure()")
}

fn tiktok_get(url: &str) -> RequestBuilder {
    get_http_client()
        .get(url)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Referer", TIKTOK_ORIGIN)
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

    // Set-Cookie (incl. tt_chain_token) is stored in the shared jar for later CDN proxy
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
