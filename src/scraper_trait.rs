use crate::models::Listing;
use anyhow::Result;
use async_trait::async_trait;

/// Trait that all scrapers must implement
#[async_trait]
pub trait Scraper: Send + Sync {
    /// Returns the name of the scraper/website
    fn name(&self) -> &str;

    /// Scrapes the website for apartment listings in the given cities
    async fn scrape(&self, cities: &[String]) -> Result<Vec<Listing>>;

    /// Returns whether this scraper is enabled
    fn is_enabled(&self) -> bool {
        true
    }
}

/// Registry to manage all scrapers
pub struct ScraperRegistry {
    scrapers: Vec<Box<dyn Scraper>>,
}

impl ScraperRegistry {
    pub fn new() -> Self {
        Self {
            scrapers: Vec::new(),
        }
    }

    pub fn register(&mut self, scraper: Box<dyn Scraper>) {
        self.scrapers.push(scraper);
    }

    pub async fn scrape_all(&self, cities: &[String]) -> Result<Vec<Listing>> {
        let mut all_listings = Vec::new();

        for scraper in &self.scrapers {
            if !scraper.is_enabled() {
                continue;
            }

            tracing::info!("Scraping from {}", scraper.name());

            match scraper.scrape(cities).await {
                Ok(mut listings) => {
                    tracing::info!("Found {} listings from {}", listings.len(), scraper.name());
                    all_listings.append(&mut listings);
                }
                Err(e) => {
                    tracing::error!("Failed to scrape from {}: {}", scraper.name(), e);
                }
            }
        }

        Ok(all_listings)
    }

    pub fn list_scrapers(&self) -> Vec<String> {
        self.scrapers.iter()
            .map(|s| s.name().to_string())
            .collect()
    }
}

impl Default for ScraperRegistry {
    fn default() -> Self {
        Self::new()
    }
}
