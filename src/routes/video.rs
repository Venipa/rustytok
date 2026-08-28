use askama::Template;
use axum::{
    extract::Path,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use crate::error::AppError;
use crate::tiktok::{self, types::VideoInfo};

#[derive(Template)]
#[template(path = "video.html")]
struct VideoTemplate {
    video: VideoInfo,
}

async fn get_video(
    Path((username, video_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let username = username.trim_start_matches('@');
    tracing::info!("Fetching video: @{} / {}", username, video_id);

    let video = tiktok::client::fetch_video(username, &video_id).await?;

    let template = VideoTemplate { video };
    Ok(Html(template.render().map_err(|_| AppError::Internal)?))
}

async fn get_video_id_only(Path(video_id): Path<String>) -> Result<Html<String>, AppError> {
    tracing::warn!(
        "Bare /video/{{}} needs username; use /@user/video/{}",
        video_id
    );
    Err(AppError::InvalidUrl)
}

pub fn router() -> Router {
    Router::new()
        .route("/:username/video/:video_id", get(get_video))
        .route("/video/:video_id", get(get_video_id_only))
}
