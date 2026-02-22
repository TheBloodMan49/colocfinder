use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Listing {
    pub id: String,
    pub title: String,
    pub price: Option<f64>,
    pub surface: Option<f64>, // Surface area in m²
    pub location: String,
    pub url: String,
    pub image_url: Option<String>, // Listing photo URL
    pub description: Option<String>,
    pub posted_at: DateTime<Utc>, // Mandatory - listings without time should be filtered out
    pub source: String,
}

impl Listing {
    /// Check if the listing has sufficient information to be displayed
    pub fn has_sufficient_info(&self) -> bool {
        // A listing should have at least a title and either a price or surface
        !self.title.trim().is_empty()
            && (self.price.is_some() || self.surface.is_some())
    }

    pub fn format_discord_message(&self) -> String {
        let mut message = format!("🏠 **New Apartment Listing**\n");
        message.push_str(&format!("**{}**\n", self.title));

        if let Some(price) = self.price {
            message.push_str(&format!("💰 Price: {:.2}€\n", price));
        }

        message.push_str(&format!("📍 Location: {}\n", self.location));

        if let Some(desc) = &self.description {
            let truncated = if desc.len() > 200 {
                format!("{}...", &desc[..200])
            } else {
                desc.clone()
            };
            message.push_str(&format!("📝 {}\n", truncated));
        }

        message.push_str(&format!("🔗 {}\n", self.url));
        message.push_str(&format!("🌐 Source: {}", self.source));

        message
    }
}
