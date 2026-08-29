use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    /// Captcha bypass cookie (`s_v_web_id`), same as ProxiTok `API_VERIFYFP`
    pub api_verify_fp: Option<String>,
    /// Device id for TikTok requests, same as ProxiTok `API_DEVICE_ID`
    pub api_device_id: Option<String>,
    /// Session cookie required for webapp-prime video CDN (`tk=tt_chain_token`)
    pub api_tt_chain_token: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            port: env::var("PORT")
                .unwrap_or_else(|_| "3000".to_string())
                .parse()
                .expect("PORT must be a number"),
            api_verify_fp: env_nonempty("API_VERIFYFP"),
            api_device_id: env_nonempty("API_DEVICE_ID"),
            api_tt_chain_token: env_nonempty("API_TT_CHAIN_TOKEN"),
        }
    }
}

fn env_nonempty(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|v| v.trim().trim_matches('"').to_string())
        .filter(|v| !v.is_empty())
}
