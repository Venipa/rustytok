use axum::{
    body::Body,
    extract::Query,
    http::{header, HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde::Deserialize;
use url::Url;

use crate::error::AppError;
use crate::tiktok::client::{get_http_client, media_cookie_header};

#[derive(Deserialize)]
pub struct ProxyQuery {
    url: String,
    /// Optional per-request tt_chain_token override
    ttc: Option<String>,
    /// Fallback filename pieces: [@user] title/id.ext
    user: Option<String>,
    title: Option<String>,
    id: Option<String>,
}

/// Proxy media (video/images) through our server to prevent TikTok tracking
async fn proxy_media(
    Query(params): Query<ProxyQuery>,
    request_headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    // Axum already percent-decodes query values; decode once more if double-encoded
    let url = urlencoding::decode(&params.url)
        .map_err(|_| AppError::InvalidUrl)?
        .into_owned();

    if !is_allowed_url(&url) {
        return Err(AppError::InvalidUrl);
    }

    tracing::debug!("Proxying media: {}", url);

    // Headers aligned with ProxiTok TikScraper\Stream
    let mut req = get_http_client()
        .get(&url)
        .header(
            header::ACCEPT,
            "video/webm,video/ogg,video/*;q=0.9,application/ogg;q=0.7,audio/*;q=0.6,*/*;q=0.5",
        )
        .header(header::ACCEPT_LANGUAGE, "en-US")
        .header(header::REFERER, "https://www.tiktok.com/")
        .header(header::ACCEPT_ENCODING, "identity")
        .header("DNT", "1")
        .header("Sec-Fetch-Dest", "video")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Site", "same-site");

    if let Some(cookie) = media_cookie_header(params.ttc.as_deref()) {
        req = req.header(header::COOKIE, cookie);
    } else if url.contains("tk=tt_chain_token") {
        tracing::warn!("CDN URL requires tt_chain_token but API_TT_CHAIN_TOKEN is unset");
    }

    if let Some(range) = request_headers.get(header::RANGE) {
        req = req.header(header::RANGE, range);
    }

    let response = req
        .send()
        .await
        .map_err(|e| AppError::FetchError(e.to_string()))?;

    let status = response.status();
    if !(status.is_success() || status.as_u16() == 206) {
        tracing::warn!("Upstream media returned {status}");
        return Err(if status.as_u16() == 404 {
            AppError::NotFound
        } else {
            AppError::FetchError(format!("CDN status: {status}"))
        });
    }

    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let content_range = response
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let content_length = response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let filename = resolve_filename(
        response.headers().get(header::CONTENT_DISPOSITION),
        &params,
        &content_type,
        &url,
    );
    let content_disposition = format!("inline; filename=\"{filename}\"");

    let stream = response.bytes_stream();
    let body = Body::from_stream(stream);

    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_DISPOSITION, content_disposition)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "public, max-age=3600");

    if let Some(cr) = content_range {
        builder = builder.header(header::CONTENT_RANGE, cr);
    }
    if let Some(cl) = content_length {
        builder = builder.header(header::CONTENT_LENGTH, cl);
    }

    Ok(builder.body(body).unwrap())
}

fn resolve_filename(
    upstream_cd: Option<&HeaderValue>,
    params: &ProxyQuery,
    content_type: &str,
    url: &str,
) -> String {
    if let Some(name) = filename_from_content_disposition(upstream_cd) {
        return name;
    }

    let ext = ext_from_content_type(content_type)
        .or_else(|| ext_from_url(url))
        .unwrap_or("bin");

    let user = params
        .user
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown");
    let label = params
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            params
                .id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })
        .unwrap_or("media");

    format!(
        "[@{}] {}.{}",
        sanitize_filename(user),
        sanitize_filename(label),
        ext
    )
}

fn filename_from_content_disposition(header: Option<&HeaderValue>) -> Option<String> {
    let raw = header?.to_str().ok()?;
    // Prefer filename*=UTF-8''... then filename="..."
    if let Some(idx) = raw.to_ascii_lowercase().find("filename*=") {
        let value = raw[idx + "filename*=".len()..].trim();
        let value = value.split(';').next()?.trim();
        let value = value.trim_matches('"');
        if let Some(encoded) = value.split("''").nth(1) {
            let decoded = urlencoding::decode(encoded).ok()?.into_owned();
            let name = sanitize_filename(&decoded);
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    if let Some(idx) = raw.to_ascii_lowercase().find("filename=") {
        let value = raw[idx + "filename=".len()..].trim();
        let value = value.split(';').next()?.trim();
        let value = value.trim_matches('"');
        let name = sanitize_filename(value);
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\n' | '\r' | '\0' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(120)
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_string()
}

fn ext_from_content_type(content_type: &str) -> Option<&'static str> {
    let mime = content_type.split(';').next()?.trim().to_ascii_lowercase();
    Some(match mime.as_str() {
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "audio/mpeg" => "mp3",
        _ => return None,
    })
}

fn ext_from_url(url: &str) -> Option<&'static str> {
    let path = Url::parse(url).ok()?.path().to_ascii_lowercase();
    if path.contains(".mp4") || url.contains("mime_type=video_mp4") {
        Some("mp4")
    } else if path.contains(".webm") {
        Some("webm")
    } else if path.contains(".jpg") || path.contains(".jpeg") {
        Some("jpg")
    } else if path.contains(".png") {
        Some("png")
    } else if path.contains(".webp") {
        Some("webp")
    } else {
        None
    }
}

fn is_allowed_url(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    let path = parsed.path();

    const SUFFIXES: &[&str] = &[
        "tiktokcdn.com",
        "tiktokcdn-us.com",
        "tiktokcdn-eu.com",
        "tiktokcdn-in.com",
        "tiktokv.com",
        "muscdn.com",
        "byteoversea.com",
        "ibytedtos.com",
    ];

    SUFFIXES
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
        || host.contains("tiktokcdn")
        || ((host == "tiktok.com" || host.ends_with(".tiktok.com"))
            && path.starts_with("/video/tos"))
}

#[cfg(test)]
mod tests {
    use super::{
        ext_from_content_type, filename_from_content_disposition, is_allowed_url, resolve_filename,
        sanitize_filename, ProxyQuery,
    };
    use axum::http::HeaderValue;

    #[test]
    fn allows_eu_tiktokcdn() {
        assert!(is_allowed_url(
            "https://p16-common-sign.tiktokcdn-eu.com/tos-useast2a-avt-0068-euttp/x.jpeg?x-signature=abc%3D"
        ));
    }

    #[test]
    fn allows_webapp_prime_video() {
        assert!(is_allowed_url(
            "https://v16-webapp-prime.tiktok.com/video/tos/no1a/tos-no1a-ve-0068c001-no/abc?a=1988"
        ));
    }

    #[test]
    fn rejects_non_cdn() {
        assert!(!is_allowed_url("https://example.com/image.jpeg"));
        assert!(!is_allowed_url("http://tiktokcdn.com/x.jpeg"));
        assert!(!is_allowed_url("https://www.tiktok.com/@user/video/123"));
        assert!(!is_allowed_url("https://www.tiktok.com/video/123"));
    }

    #[test]
    fn fallback_filename_uses_user_and_title() {
        let params = ProxyQuery {
            url: String::new(),
            ttc: None,
            user: Some("zushi".into()),
            title: Some("cool clip".into()),
            id: Some("123".into()),
        };
        assert_eq!(
            resolve_filename(None, &params, "video/mp4", "https://x/video.mp4"),
            "[@zushi] cool clip.mp4"
        );
    }

    #[test]
    fn fallback_filename_uses_id_when_no_title() {
        let params = ProxyQuery {
            url: String::new(),
            ttc: None,
            user: Some("zushi".into()),
            title: None,
            id: Some("7677".into()),
        };
        assert_eq!(
            resolve_filename(None, &params, "video/mp4", "https://x"),
            "[@zushi] 7677.mp4"
        );
    }

    #[test]
    fn prefers_upstream_content_disposition() {
        let cd = HeaderValue::from_static("inline; filename=\"from-cdn.mp4\"");
        let params = ProxyQuery {
            url: String::new(),
            ttc: None,
            user: Some("zushi".into()),
            title: Some("ignored".into()),
            id: None,
        };
        assert_eq!(
            resolve_filename(Some(&cd), &params, "video/mp4", "https://x"),
            "from-cdn.mp4"
        );
    }

    #[test]
    fn parses_quoted_filename() {
        let cd = HeaderValue::from_static("attachment; filename=\"a b.mp4\"");
        assert_eq!(
            filename_from_content_disposition(Some(&cd)).as_deref(),
            Some("a b.mp4")
        );
    }

    #[test]
    fn sanitizes_path_chars() {
        assert_eq!(sanitize_filename("a/b:c?.mp4"), "a_b_c_.mp4");
    }

    #[test]
    fn maps_content_type_ext() {
        assert_eq!(
            ext_from_content_type("video/mp4; charset=binary"),
            Some("mp4")
        );
    }
}

pub fn router() -> Router {
    Router::new().route("/proxy", get(proxy_media))
}
