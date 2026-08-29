use std::sync::{Arc, Mutex};

use once_cell::sync::OnceCell;
use reqwest::cookie::Jar;
use reqwest::header::{HeaderMap, SET_COOKIE};
use reqwest::{Client, RequestBuilder, Url};

use crate::config::Config;
use crate::error::AppError;
use super::parser;
use super::types::{UserInfo, VideoInfo, TagInfo};

static HTTP_CLIENT: OnceCell<Client> = OnceCell::new();
/// Cookies sent on CDN media requests (explicit header; jar Domain matching is flaky)
static MEDIA_COOKIES: OnceCell<Mutex<String>> = OnceCell::new();

const TIKTOK_ORIGIN: &str = "https://www.tiktok.com";

pub fn configure(config: &Config) {
    let jar = Arc::new(Jar::default());
    let origin: Url = TIKTOK_ORIGIN.parse().expect("valid tiktok origin");
    let cdn: Url = "https://v16-webapp-prime.tiktok.com/"
        .parse()
        .expect("valid cdn origin");

    let mut media_parts: Vec<String> = Vec::new();

    if let Some(fp) = config.api_verify_fp.as_deref().map(trim_cookie_value) {
        seed_cookie(&jar, "s_v_web_id", &fp, &origin);
        seed_cookie(&jar, "s_v_web_id", &fp, &cdn);
        media_parts.push(format!("s_v_web_id={fp}"));
    }
    if let Some(id) = config.api_device_id.as_deref().map(trim_cookie_value) {
        seed_cookie(&jar, "tt_webid", &id, &origin);
        seed_cookie(&jar, "tt_webid_v2", &id, &origin);
        media_parts.push(format!("tt_webid={id}"));
        media_parts.push(format!("tt_webid_v2={id}"));
    }
    if let Some(token) = config.api_tt_chain_token.as_deref().map(trim_cookie_value) {
        // Quote in Set-Cookie form so base64 `=` / `/` are not truncated by Cookie::parse
        seed_cookie(&jar, "tt_chain_token", &token, &origin);
        seed_cookie(&jar, "tt_chain_token", &token, &cdn);
        media_parts.push(format!("tt_chain_token={token}"));
        tracing::info!("API_TT_CHAIN_TOKEN loaded (len={})", token.len());
    }

    let _ = MEDIA_COOKIES.set(Mutex::new(media_parts.join("; ")));

    let client = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .cookie_provider(jar)
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("Failed to create HTTP client");

    let _ = HTTP_CLIENT.set(client);
}

fn trim_cookie_value(value: &str) -> String {
    value.trim().trim_matches('"').to_string()
}

fn seed_cookie(jar: &Jar, name: &str, value: &str, url: &Url) {
    jar.add_cookie_str(
        &format!("{name}=\"{value}\"; Domain=.tiktok.com; Path=/; Secure"),
        url,
    );
}

fn ingest_set_cookie(headers: &HeaderMap) {
    for value in headers.get_all(SET_COOKIE) {
        let Ok(raw) = value.to_str() else {
            continue;
        };
        if let Some(token) = raw
            .split(';')
            .next()
            .and_then(|pair| pair.strip_prefix("tt_chain_token="))
            .map(trim_cookie_value)
            .filter(|v| !v.is_empty())
        {
            tracing::debug!("Captured tt_chain_token from Set-Cookie (len={})", token.len());
            update_media_tt_chain_token(&token);
        }
    }
}

fn update_media_tt_chain_token(token: &str) {
    let Some(lock) = MEDIA_COOKIES.get() else {
        return;
    };
    let mut cookies = lock.lock().expect("media cookies lock");
    let without = cookies
        .split("; ")
        .filter(|p| !p.is_empty() && !p.starts_with("tt_chain_token="))
        .collect::<Vec<_>>()
        .join("; ");
    *cookies = if without.is_empty() {
        format!("tt_chain_token={token}")
    } else {
        format!("{without}; tt_chain_token={token}")
    };
}

pub fn get_http_client() -> &'static Client {
    HTTP_CLIENT
        .get()
        .expect("HTTP client used before configure()")
}

/// Cookie header for CDN fetches (`tt_chain_token` required when URL has `tk=tt_chain_token`)
pub fn media_cookie_header(override_ttc: Option<&str>) -> Option<String> {
    let base = MEDIA_COOKIES
        .get()
        .and_then(|lock| lock.lock().ok().map(|g| g.clone()))
        .unwrap_or_default();
    let override_ttc = override_ttc
        .map(trim_cookie_value)
        .filter(|v| !v.is_empty());

    match (base.is_empty(), override_ttc) {
        (true, None) => None,
        (true, Some(ttc)) => Some(format!("tt_chain_token={ttc}")),
        (false, None) => Some(base),
        (false, Some(ttc)) => {
            let without = base
                .split("; ")
                .filter(|p| !p.starts_with("tt_chain_token="))
                .collect::<Vec<_>>()
                .join("; ");
            if without.is_empty() {
                Some(format!("tt_chain_token={ttc}"))
            } else {
                Some(format!("{without}; tt_chain_token={ttc}"))
            }
        }
    }
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

    ingest_set_cookie(response.headers());

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

    ingest_set_cookie(response.headers());

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

    ingest_set_cookie(response.headers());

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
    use super::{media_cookie_header, trim_cookie_value};

    #[test]
    fn strips_quotes_from_cookie_value() {
        assert_eq!(trim_cookie_value("\"abc==\""), "abc==");
    }

    #[test]
    fn media_cookie_override_replaces_ttc() {
        let header = media_cookie_header(Some("fresh==token"));
        assert_eq!(header.as_deref(), Some("tt_chain_token=fresh==token"));
    }
}
