use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: String,
    pub username: String,
    pub nickname: String,
    pub bio: String,
    pub avatar_url: String,
    pub follower_count: u64,
    pub following_count: u64,
    pub like_count: u64,
    pub video_count: u64,
    pub videos: Vec<VideoInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoInfo {
    pub id: String,
    pub description: String,
    pub author_username: String,
    pub author_nickname: String,
    pub author_avatar: String,
    pub video_url: String,
    pub thumbnail_url: String,
    pub like_count: u64,
    pub comment_count: u64,
    pub share_count: u64,
    pub view_count: u64,
    pub create_time: i64,
    pub music_title: Option<String>,
    pub music_author: Option<String>,
}

impl VideoInfo {
    /// Get proxied video URL (includes fallback download filename metadata)
    pub fn proxied_video_url(&self) -> String {
        let mut url = format!("/proxy?url={}", urlencoding::encode(&self.video_url));
        if !self.author_username.is_empty() {
            url.push_str(&format!(
                "&user={}",
                urlencoding::encode(&self.author_username)
            ));
        }
        let title: String = self
            .description
            .chars()
            .take(80)
            .collect::<String>()
            .trim()
            .to_string();
        if !title.is_empty() {
            url.push_str(&format!("&title={}", urlencoding::encode(&title)));
        }
        if !self.id.is_empty() {
            url.push_str(&format!("&id={}", urlencoding::encode(&self.id)));
        }
        url
    }

    /// Get proxied thumbnail URL
    pub fn proxied_thumbnail_url(&self) -> String {
        let mut url = format!("/proxy?url={}", urlencoding::encode(&self.thumbnail_url));
        if !self.author_username.is_empty() {
            url.push_str(&format!(
                "&user={}",
                urlencoding::encode(&self.author_username)
            ));
        }
        if !self.id.is_empty() {
            url.push_str(&format!(
                "&title={}",
                urlencoding::encode(&format!("{}-thumb", self.id))
            ));
            url.push_str(&format!("&id={}", urlencoding::encode(&self.id)));
        }
        url
    }
}

impl UserInfo {
    /// Get proxied avatar URL
    pub fn proxied_avatar_url(&self) -> String {
        let mut url = format!("/proxy?url={}", urlencoding::encode(&self.avatar_url));
        if !self.username.is_empty() {
            url.push_str(&format!("&user={}", urlencoding::encode(&self.username)));
        }
        url.push_str("&title=avatar");
        if !self.id.is_empty() {
            url.push_str(&format!("&id={}", urlencoding::encode(&self.id)));
        }
        url
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagInfo {
    pub name: String,
    pub view_count: u64,
    pub videos: Vec<VideoInfo>,
}
