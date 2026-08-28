use axum::{
    body::Body,
    extract::Query,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde::Deserialize;
use url::Url;

use crate::error::AppError;
use crate::tiktok::client::get_http_client;

#[derive(Deserialize)]
pub struct ProxyQuery {
    url: String,
}

/// Proxy media (video/images) through our server to prevent TikTok tracking
async fn proxy_media(Query(params): Query<ProxyQuery>) -> Result<impl IntoResponse, AppError> {
    // Axum already percent-decodes query values; decode once more if double-encoded
    let url = urlencoding::decode(&params.url)
        .map_err(|_| AppError::InvalidUrl)?
        .into_owned();

    if !is_allowed_url(&url) {
        return Err(AppError::InvalidUrl);
    }

    tracing::debug!("Proxying media: {}", url);

    let client = get_http_client();
    let response = client
        .get(&url)
        .header(header::REFERER, "https://www.tiktok.com/")
        .header(header::ORIGIN, "https://www.tiktok.com")
        .header(header::ACCEPT, "*/*")
        .send()
        .await
        .map_err(|e| AppError::FetchError(e.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        tracing::warn!("Upstream media {} returned {}", url, status);
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

    let stream = response.bytes_stream();
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "public, max-age=86400")
        .body(body)
        .unwrap())
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

    // Exact / subdomain match for known CDNs
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
        // Regional variants like p16-common-sign.tiktokcdn-eu.com
        || host.contains("tiktokcdn")
        // Stream hosts e.g. v16-webapp-prime.tiktok.com/video/tos/...
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
    Router::new()
        .route("/proxy", get(proxy_media))
}
