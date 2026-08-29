use axum::{
    body::Body,
    extract::Query,
    http::{header, HeaderMap, StatusCode},
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

    let stream = response.bytes_stream();
    let body = Body::from_stream(stream);

    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
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
    use super::is_allowed_url;

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
}

pub fn router() -> Router {
    Router::new().route("/proxy", get(proxy_media))
}
