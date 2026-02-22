use crate::models::Listing;
use std::collections::HashSet;
use std::fs;
use anyhow::Result;

/// Tracks seen listings to avoid duplicate notifications
pub struct ListingTracker {
    seen_ids: HashSet<String>,
    cache_file: String,
}

impl ListingTracker {
    pub fn new(cache_file: &str) -> Self {
        let seen_ids = Self::load_from_file(cache_file).unwrap_or_default();
        
        Self {
            seen_ids,
            cache_file: cache_file.to_string(),
        }
    }
    
    fn load_from_file(path: &str) -> Result<HashSet<String>> {
        let content = fs::read_to_string(path)?;
        let ids: HashSet<String> = serde_json::from_str(&content)?;
        Ok(ids)
    }
    
    fn save_to_file(&self) -> Result<()> {
        let content = serde_json::to_string(&self.seen_ids)?;
        fs::write(&self.cache_file, content)?;
        Ok(())
    }
    
    /// Filters out listings that have already been seen
    pub fn filter_new(&mut self, listings: Vec<Listing>) -> Vec<Listing> {
        let new_listings: Vec<Listing> = listings
            .into_iter()
            .filter(|listing| !self.seen_ids.contains(&listing.id))
            .collect();
        
        // Mark new listings as seen
        for listing in &new_listings {
            self.seen_ids.insert(listing.id.clone());
        }
        
        // Save to file
        if let Err(e) = self.save_to_file() {
            tracing::warn!("Failed to save tracker cache: {}", e);
        }
        
        new_listings
    }
    
    #[allow(dead_code)]
    pub fn clear(&mut self) -> Result<()> {
        self.seen_ids.clear();
        self.save_to_file()
    }
    
    #[allow(dead_code)]
    pub fn count(&self) -> usize {
        self.seen_ids.len()
    }
}
