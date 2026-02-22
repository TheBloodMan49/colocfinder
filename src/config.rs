use serde::{Deserialize, Serialize};
use std::fs;
use std::env;
use anyhow::{Result, Context};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub discord_token: String,
    pub channel_id: u64,
    pub interesting_channel_id: u64,
    pub check_interval_seconds: u64,
    pub cities: Vec<String>,
    #[serde(default = "default_tracing_level")]
    pub tracing_level: String,
    #[serde(default = "default_user_agent")]
    pub user_agent: String,
    #[serde(default = "default_request_delay_ms")]
    pub request_delay_ms: u64,
    #[serde(default = "default_max_listing_age_minutes")]
    pub max_listing_age_minutes: u64,
    #[serde(default = "default_min_rooms")]
    pub min_rooms: u32,
}

fn default_tracing_level() -> String {
    "info".to_string()
}

fn default_user_agent() -> String {
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36".to_string()
}

fn default_request_delay_ms() -> u64 {
    2000 // 2 seconds between requests
}

fn default_max_listing_age_minutes() -> u64 {
    1440 // 24 hours by default
}

fn default_min_rooms() -> u32 {
    1 // Accept all listings by default
}

impl Config {
    pub fn load() -> Result<Self> {
        // Try to load from file first, or use defaults
        // Check data/config.yaml first, then fallback to config.yaml for backwards compatibility
        let config_path = "data/config.yaml";

        let mut config: Config = if let Ok(config_str) = fs::read_to_string(config_path) {
            serde_yaml::from_str(&config_str)?
        } else {
            // Create a minimal default config if file doesn't exist
            Config {
                discord_token: String::new(),
                channel_id: 0,
                interesting_channel_id: 0,
                check_interval_seconds: 300,
                cities: vec![],
                tracing_level: default_tracing_level(),
                user_agent: default_user_agent(),
                request_delay_ms: default_request_delay_ms(),
                max_listing_age_minutes: default_max_listing_age_minutes(),
                min_rooms: default_min_rooms(),
            }
        };

        // Override with environment variables if present
        if let Ok(token) = env::var("DISCORD_TOKEN") {
            config.discord_token = token;
        }

        if let Ok(channel_id) = env::var("CHANNEL_ID") {
            config.channel_id = channel_id.parse()
                .context("Failed to parse CHANNEL_ID environment variable")?;
        }

        if let Ok(interesting_channel_id) = env::var("INTERESTING_CHANNEL_ID") {
            config.interesting_channel_id = interesting_channel_id.parse()
                .context("Failed to parse INTERESTING_CHANNEL_ID environment variable")?;
        }

        if let Ok(check_interval) = env::var("CHECK_INTERVAL_SECONDS") {
            config.check_interval_seconds = check_interval.parse()
                .context("Failed to parse CHECK_INTERVAL_SECONDS environment variable")?;
        }

        if let Ok(cities) = env::var("CITIES") {
            // Parse comma-separated cities
            config.cities = cities.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }

        if let Ok(tracing_level) = env::var("TRACING_LEVEL") {
            config.tracing_level = tracing_level;
        }

        if let Ok(user_agent) = env::var("USER_AGENT") {
            config.user_agent = user_agent;
        }

        if let Ok(request_delay) = env::var("REQUEST_DELAY_MS") {
            config.request_delay_ms = request_delay.parse()
                .context("Failed to parse REQUEST_DELAY_MS environment variable")?;
        }

        if let Ok(max_age) = env::var("MAX_LISTING_AGE_MINUTES") {
            config.max_listing_age_minutes = max_age.parse()
                .context("Failed to parse MAX_LISTING_AGE_MINUTES environment variable")?;
        }

        if let Ok(min_rooms) = env::var("MIN_ROOMS") {
            config.min_rooms = min_rooms.parse()
                .context("Failed to parse MIN_ROOMS environment variable")?;
        }

        // Validate required fields
        if config.discord_token.is_empty() {
            anyhow::bail!("discord_token is required (set via data/config.yaml or DISCORD_TOKEN env var)");
        }

        if config.channel_id == 0 {
            anyhow::bail!("channel_id is required (set via data/config.yaml or CHANNEL_ID env var)");
        }

        if config.interesting_channel_id == 0 {
            anyhow::bail!("interesting_channel_id is required (set via data/config.yaml or INTERESTING_CHANNEL_ID env var)");
        }

        if config.cities.is_empty() {
            anyhow::bail!("At least one city is required (set via data/config.yaml or CITIES env var)");
        }

        Ok(config)
    }

    pub fn create_default() -> Result<()> {
        // Ensure data directory exists
        std::fs::create_dir_all("data")?;

        let default_config = Config {
            discord_token: "YOUR_DISCORD_BOT_TOKEN".to_string(),
            channel_id: 0,
            interesting_channel_id: 0,
            check_interval_seconds: 300, // 5 minutes
            cities: vec!["Paris".to_string(), "Lyon".to_string()],
            tracing_level: "info".to_string(),
            user_agent: default_user_agent(),
            request_delay_ms: 2000,
            max_listing_age_minutes: 1440, // 24 hours
            min_rooms: 1,
        };

        let config_str = serde_yaml::to_string(&default_config)?;
        fs::write("data/config.yaml", config_str)?;
        Ok(())
    }
}
