use crate::http_client;
use crate::models::Listing;
use crate::scraper_trait::Scraper;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc, Duration, NaiveDateTime, TimeZone};
use scraper::{Html, Selector};
use std::sync::Arc;
use reqwest::cookie::Jar;

pub struct LeboncoinScraper {
    client: reqwest::Client,
    request_delay_ms: u64,
    max_listing_age_minutes: u64,
    min_rooms: u32,
    cookie_jar: Arc<Jar>,
}

impl LeboncoinScraper {
    pub fn new() -> Self {
        Self::with_config(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            2000,
            1440, // 24 hours default
            1 // Accept all listings by default
        )
    }

    pub fn with_config(user_agent: &str, request_delay_ms: u64, max_listing_age_minutes: u64, min_rooms: u32) -> Self {
        // Create a persistent cookie jar
        let cookie_jar = Arc::new(Jar::default());

        Self {
            client: http_client::create_http_client_with_cookies(user_agent, Some(cookie_jar.clone()))
                .unwrap_or_else(|_| reqwest::Client::new()),
            request_delay_ms,
            max_listing_age_minutes,
            min_rooms,
            cookie_jar,
        }
    }

    /// Get the cookie jar for inspection or manual cookie management
    pub fn cookie_jar(&self) -> &Arc<Jar> {
        &self.cookie_jar
    }

    /// Load cookies from a JSON file exported from browser
    /// Expected format: Array of cookies with "name", "value", "domain" fields
    /// You can export cookies using browser extensions like "EditThisCookie"
    pub fn load_cookies_from_file(&self, path: &str) -> Result<()> {
        use std::fs;

        let cookie_data = fs::read_to_string(path)?;
        let cookies: Vec<serde_json::Value> = serde_json::from_str(&cookie_data)?;

        let leboncoin_url = "https://www.leboncoin.fr".parse::<reqwest::Url>()
            .expect("Invalid leboncoin URL");

        let mut loaded_count = 0;
        for cookie in &cookies {
            if let (Some(name), Some(value)) = (cookie.get("name"), cookie.get("value")) {
                let name = name.as_str().unwrap_or("");
                let value = value.as_str().unwrap_or("");

                // Format as "name=value" cookie string
                let cookie_str = format!("{}={}", name, value);
                self.cookie_jar.add_cookie_str(&cookie_str, &leboncoin_url);

                tracing::debug!("Loaded cookie: {}", name);
                loaded_count += 1;
            }
        }

        tracing::info!("Loaded {} cookies from {}", loaded_count, path);
        Ok(())
    }

    fn build_search_url(&self, city: &str) -> String {
        // Map city names to Leboncoin location parameters
        // Format: CITY_POSTALCODE__LATITUDE_LONGITUDE_RADIUS_RADIUS
        let location = match city.to_uppercase().as_str() {
            "RENNES" => "RENNES_35000__48.10824_-1.68449_5000_5000",
            "PARIS" => "PARIS_75000__48.856614_2.3522219_5000_5000",
            "LYON" => "LYON_69000__45.764043_4.835659_5000_5000",
            "MARSEILLE" => "MARSEILLE_13000__43.296482_5.36978_5000_5000",
            "TOULOUSE" => "TOULOUSE_31000__43.604652_1.444209_5000_5000",
            "NICE" => "NICE_06000__43.710173_7.261953_5000_5000",
            "NANTES" => "NANTES_44000__47.218371_-1.553621_5000_5000",
            "BORDEAUX" => "BORDEAUX_33000__44.837789_-0.57918_5000_5000",
            "LILLE" => "LILLE_59000__50.62925_3.057256_5000_5000",
            "STRASBOURG" => "STRASBOURG_67000__48.573405_7.752111_5000_5000",
            _ => {
                // Fallback to simple city name search
                tracing::warn!("No location coordinates configured for city '{}', using simple search", city);
                return format!(
                    "https://www.leboncoin.fr/recherche?category=10&locations={}&real_estate_type=2&sort=time&order=desc",
                    urlencoding::encode(city)
                );
            }
        };

        format!(
            "https://www.leboncoin.fr/recherche?category=10&locations={}&real_estate_type=2&sort=time&order=desc",
            location
        )
    }

    /// Parse price from text (e.g., "850 €", "1 200 €", "850,50 €")
    fn parse_price(price_text: &str) -> Option<f64> {
        if price_text.is_empty() {
            return None;
        }

        price_text
            .replace("€", "")
            .replace(" ", "")
            .replace(",", ".")
            .replace("\u{00a0}", "") // non-breaking space
            .trim()
            .parse::<f64>()
            .ok()
    }

    /// Extract surface from title (e.g., "28 mètres carrés" or "28m²")
    fn parse_surface(title: &str) -> Option<f64> {
        if title.is_empty() {
            return None;
        }

        let surface_regex = regex::Regex::new(r"(\d+)\s*(?:mètres carrés|m²|métres carres)").ok()?;
        surface_regex
            .captures(title)
            .and_then(|caps| caps.get(1))
            .and_then(|m| m.as_str().parse::<f64>().ok())
    }

    /// Extract number of rooms from title (e.g., "T2", "T3", "2 pièces", "3 chambres", "F2")
    /// Returns the number of rooms if found
    fn parse_rooms(title: &str) -> Option<u32> {
        if title.is_empty() {
            return None;
        }

        // Try multiple patterns commonly used in French real estate listings
        // T1, T2, T3, etc. (studio is T1)
        // F1, F2, F3, etc. (same as T)
        // "2 pièces", "3 pièces", etc.
        // "2 chambres", "3 chambres", etc.

        // Pattern for T1-T9 or F1-F9
        if let Ok(t_regex) = regex::Regex::new(r"\b[TF](\d)\b") {
            if let Some(caps) = t_regex.captures(title) {
                if let Some(num) = caps.get(1).and_then(|m| m.as_str().parse::<u32>().ok()) {
                    return Some(num);
                }
            }
        }

        // Pattern for "X pièces" or "X pieces" or "X pièce" (singular)
        if let Ok(pieces_regex) = regex::Regex::new(r"(\d+)\s*pi[èe]ces?") {
            if let Some(caps) = pieces_regex.captures(title) {
                if let Some(num) = caps.get(1).and_then(|m| m.as_str().parse::<u32>().ok()) {
                    return Some(num);
                }
            }
        }

        // Pattern for "X chambres"
        if let Ok(chambres_regex) = regex::Regex::new(r"(\d+)\s*chambres?") {
            if let Some(caps) = chambres_regex.captures(title) {
                if let Some(num) = caps.get(1).and_then(|m| m.as_str().parse::<u32>().ok()) {
                    // Add 1 because "X chambres" usually means X bedrooms + 1 living room
                    return Some(num + 1);
                }
            }
        }

        None
    }

    /// Convert relative URL to absolute URL
    fn build_full_url(relative_url: &str) -> String {
        if relative_url.starts_with("http") {
            relative_url.to_string()
        } else if !relative_url.is_empty() {
            format!("https://www.leboncoin.fr{}", relative_url)
        } else {
            String::new()
        }
    }

    /// Extract ID from URL
    fn extract_id_from_url(full_url: &str, fallback: &str) -> String {
        if full_url.is_empty() {
            return fallback.to_string();
        }

        full_url
            .split('/')
            .last()
            .and_then(|s| s.split('.').next())
            .unwrap_or(fallback)
            .to_string()
    }

    /// Extract title from an HTML element
    fn extract_title(element: &scraper::ElementRef) -> String {
        let title_selectors = vec![
            "p[data-qa-id='aditem_title']",
            "div[data-qa-id='aditem_title']",
            "span[data-qa-id='aditem_title']",
            ".styles_adCard__title__HpiGb",
            "h2",
            "h3",
        ];

        title_selectors.iter()
            .find_map(|sel_str| {
                Selector::parse(sel_str).ok()
                    .and_then(|sel| element.select(&sel).next())
                    .map(|el| el.text().collect::<String>())
            })
            .or_else(|| {
                element.value().attr("aria-label").map(|s| s.to_string())
            })
            .unwrap_or_default()
    }

    /// Extract price text from an HTML element
    fn extract_price_text(element: &scraper::ElementRef) -> String {
        let price_selectors = vec![
            "p[data-test-id='price']",
            "div[data-test-id='price']",
            "span[data-test-id='price']",
            "p[data-qa-id='aditem_price']",
            "span[data-qa-id='aditem_price']",
        ];

        price_selectors.iter()
            .find_map(|sel_str| {
                Selector::parse(sel_str).ok()
                    .and_then(|sel| element.select(&sel).next())
                    .map(|el| {
                        // Try to get text from nested span first
                        if let Ok(span_sel) = Selector::parse("span") {
                            if let Some(span_el) = el.select(&span_sel).next() {
                                return span_el.text().collect::<String>();
                            }
                        }
                        el.text().collect::<String>()
                    })
            })
            .unwrap_or_default()
    }

    /// Extract image URL from an HTML element
    fn extract_image_url(element: &scraper::ElementRef) -> Option<String> {
        let image_selectors = vec![
            "img[src*='leboncoin.fr']",
            "img[data-test-id='adcard-image']",
            "img",
        ];

        image_selectors.iter()
            .find_map(|sel_str| {
                Selector::parse(sel_str).ok()
                    .and_then(|sel| element.select(&sel).next())
                    .and_then(|el| el.value().attr("src"))
                    .filter(|src| src.contains("leboncoin.fr") && !src.is_empty())
                    .map(|src| src.to_string())
            })
    }

    /// Extract relative URL from an HTML element
    fn extract_relative_url(element: &scraper::ElementRef) -> String {
        let link_selectors = vec!["a"];

        link_selectors.iter()
            .find_map(|sel_str| {
                Selector::parse(sel_str).ok()
                    .and_then(|sel| element.select(&sel).next())
                    .and_then(|el| el.value().attr("href"))
            })
            .or_else(|| element.value().attr("href"))
            .unwrap_or("")
            .to_string()
    }

    /// Extract posted_at time from the p tag's title attribute
    /// The title contains the full datetime like "Aujourd'hui, 14:30" or "13 février 2026, 10:15"
    fn extract_posted_at(element: &scraper::ElementRef) -> Option<DateTime<Utc>> {
        // Look for p tags with time information
        let time_selectors = vec![
            "p[title]",
            "time[datetime]",
        ];

        tracing::trace!("Looking for posted_at time in element...");

        for sel_str in time_selectors {
            if let Ok(selector) = Selector::parse(sel_str) {
                let matches: Vec<_> = element.select(&selector).collect();
                tracing::trace!("Selector '{}' found {} matches", sel_str, matches.len());

                for (idx, time_element) in matches.iter().enumerate() {
                    // First try datetime attribute (if it's a time tag)
                    if let Some(datetime_str) = time_element.value().attr("datetime") {
                        tracing::trace!("Match #{}: Found datetime attribute: {}", idx, datetime_str);
                        if let Ok(dt) = DateTime::parse_from_rfc3339(datetime_str) {
                            tracing::debug!("Successfully parsed datetime from attribute: {}", dt);
                            return Some(dt.with_timezone(&Utc));
                        }
                    }

                    // Try title attribute
                    if let Some(title) = time_element.value().attr("title") {
                        tracing::trace!("Match #{}: Found time title attribute: '{}'", idx, title);
                        if let Some(dt) = Self::parse_french_datetime(title) {
                            tracing::debug!("✓ Successfully parsed French datetime from title: {}", dt);
                            return Some(dt);
                        } else {
                            tracing::trace!("Match #{}: Failed to parse French datetime: '{}'", idx, title);
                        }
                    }

                    // Try text content as fallback
                    let text: String = time_element.text().collect();
                    if !text.trim().is_empty() {
                        tracing::trace!("Match #{}: Trying to parse time from text: '{}'", idx, text.trim());
                        if let Some(dt) = Self::parse_french_datetime(&text) {
                            tracing::debug!("✓ Successfully parsed French datetime from text: {}", dt);
                            return Some(dt);
                        }
                    }
                }
            }
        }

        tracing::warn!("⚠ No posted_at time found in element");
        None
    }

    /// Parse French datetime strings like:
    /// - "Aujourd'hui, 14:30"
    /// - "Hier, 10:15"
    /// - "13 février 2026, 10:15"
    fn parse_french_datetime(datetime_str: &str) -> Option<DateTime<Utc>> {
        let now = Utc::now();
        let today = now.date_naive();

        // Handle "Aujourd'hui, HH:MM"
        if datetime_str.starts_with("Aujourd'hui") || datetime_str.starts_with("aujourd'hui") {
            if let Some(time_str) = datetime_str.split(',').nth(1) {
                return Self::parse_time_today(time_str.trim(), today);
            }
        }

        // Handle "Hier, HH:MM"
        if datetime_str.starts_with("Hier") || datetime_str.starts_with("hier") {
            if let Some(time_str) = datetime_str.split(',').nth(1) {
                let yesterday = today - Duration::days(1);
                return Self::parse_time_today(time_str.trim(), yesterday);
            }
        }

        // Handle full date format: "13 février 2026, 10:15"
        // This is more complex and would require month name mapping
        if let Some(dt) = Self::parse_full_french_date(datetime_str) {
            return Some(dt);
        }

        None
    }

    /// Parse time string (HH:MM) for a given date
    fn parse_time_today(time_str: &str, date: chrono::NaiveDate) -> Option<DateTime<Utc>> {
        let parts: Vec<&str> = time_str.split(':').collect();
        if parts.len() == 2 {
            if let (Ok(hour), Ok(minute)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                if let Some(naive_time) = chrono::NaiveTime::from_hms_opt(hour, minute, 0) {
                    let naive_datetime = NaiveDateTime::new(date, naive_time);
                    // Assume French time (UTC+1 or UTC+2 depending on DST)
                    // For simplicity, we'll use UTC+1
                    let paris_offset = chrono::FixedOffset::east_opt(3600)?;
                    let paris_dt = paris_offset.from_local_datetime(&naive_datetime).single()?;
                    return Some(paris_dt.with_timezone(&Utc));
                }
            }
        }
        None
    }

    /// Parse full French date format: "13 février 2026, 10:15" or "13 février 2026 à 10:15"
    fn parse_full_french_date(datetime_str: &str) -> Option<DateTime<Utc>> {
        // Month mapping
        let months = [
            ("janvier", 1), ("février", 2), ("mars", 3), ("avril", 4),
            ("mai", 5), ("juin", 6), ("juillet", 7), ("août", 8),
            ("septembre", 9), ("octobre", 10), ("novembre", 11), ("décembre", 12),
        ];

        // Parse format: "DD month YYYY, HH:MM" or "DD month YYYY à HH:MM"
        // Try splitting by comma first, then by "à" (French for "at")
        let parts: Vec<&str> = if datetime_str.contains(',') {
            datetime_str.split(',').collect()
        } else if datetime_str.contains(" à ") {
            datetime_str.split(" à ").collect()
        } else {
            return None;
        };

        if parts.len() != 2 {
            return None;
        }

        let date_part = parts[0].trim();
        let time_part = parts[1].trim();

        // Split date part into day, month, year
        let date_components: Vec<&str> = date_part.split_whitespace().collect();
        if date_components.len() != 3 {
            return None;
        }

        let day: u32 = date_components[0].parse().ok()?;
        let month_str = date_components[1].to_lowercase();
        let year: i32 = date_components[2].parse().ok()?;

        let month = months.iter()
            .find(|(name, _)| *name == month_str)
            .map(|(_, m)| *m)?;

        // Parse time
        let time_components: Vec<&str> = time_part.split(':').collect();
        if time_components.len() != 2 {
            return None;
        }

        let hour: u32 = time_components[0].parse().ok()?;
        let minute: u32 = time_components[1].parse().ok()?;

        let naive_date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
        let naive_time = chrono::NaiveTime::from_hms_opt(hour, minute, 0)?;
        let naive_datetime = NaiveDateTime::new(naive_date, naive_time);

        // Assume French time (UTC+1)
        let paris_offset = chrono::FixedOffset::east_opt(3600)?;
        let paris_dt = paris_offset.from_local_datetime(&naive_datetime).single()?;
        Some(paris_dt.with_timezone(&Utc))
    }
}

#[async_trait]
impl Scraper for LeboncoinScraper {
    fn name(&self) -> &str {
        "Leboncoin"
    }

    async fn scrape(&self, cities: &[String]) -> Result<Vec<Listing>> {
        let mut listings = Vec::new();

        for city in cities {
            let url = self.build_search_url(city);
            tracing::debug!("Scraping {}", url);

            match self.client.get(&url).send().await {
                Ok(response) => {
                    let html = response.text().await?;
                    tracing::debug!("Fetched HTML content for {}: {} bytes", city, html.len());

                    // Save HTML to file for debugging if needed
                    if tracing::enabled!(tracing::Level::TRACE) {
                        if let Err(e) = std::fs::write(format!("debug_{}.html", city), &html) {
                            tracing::warn!("Failed to write debug HTML: {}", e);
                        }
                    }

                    let document = Html::parse_document(&html);

                    // Leboncoin uses <article> tags for each listing
                    // Try multiple possible selectors
                    let possible_selectors = vec![
                        "article[data-qa-id='aditem']",
                        "article",
                        "div[data-qa-id='aditem']",
                        "a[data-qa-id='aditem_container']",
                    ];

                    let mut found_selector = None;
                    let mut found_selector_str = "";
                    for selector_str in possible_selectors {
                        if let Ok(selector) = Selector::parse(selector_str) {
                            let count = document.select(&selector).count();
                            if count > 0 {
                                tracing::debug!("Found {} elements with selector: {}", count, selector_str);
                                found_selector = Some(selector);
                                found_selector_str = selector_str;
                                break;
                            } else {
                                tracing::trace!("Selector '{}' found 0 elements", selector_str);
                            }
                        }
                    }

                    let mut listings_for_city = 0;
                    let mut filtered_by_age = 0;
                    let mut filtered_by_rooms = 0;
                    let now = Utc::now();
                    let max_age = Duration::minutes(self.max_listing_age_minutes as i64);

                    if let Some(listing_selector) = found_selector {
                        tracing::info!("Using selector: '{}'", found_selector_str);
                        for (index, element) in document.select(&listing_selector).enumerate() {
                            tracing::trace!("Processing listing #{}", index + 1);

                            // Extract posted_at time - MANDATORY
                            let posted_at = match Self::extract_posted_at(&element) {
                                Some(time) => time,
                                None => {
                                    tracing::warn!("Listing #{} - no posted_at time found, skipping", index + 1);
                                    continue;
                                }
                            };

                            // Filter by age
                            let age = now.signed_duration_since(posted_at);
                            tracing::debug!("Listing #{}: posted at {}, age: {} minutes (max: {} minutes)",
                                index + 1, posted_at, age.num_minutes(), self.max_listing_age_minutes);
                            if age > max_age {
                                tracing::debug!("Skipping listing #{} - too old (age: {} minutes, max: {} minutes)",
                                    index + 1, age.num_minutes(), self.max_listing_age_minutes);
                                filtered_by_age += 1;
                                continue;
                            }

                            // Extract title
                            let title = Self::extract_title(&element);

                            // Extract number of rooms and filter if needed
                            let rooms = Self::parse_rooms(&title);
                            if self.min_rooms > 1 {
                                if let Some(room_count) = rooms {
                                    if room_count < self.min_rooms {
                                        tracing::debug!("Skipping listing #{} - not enough rooms ({} < {}): {}",
                                            index + 1, room_count, self.min_rooms, title);
                                        filtered_by_rooms += 1;
                                        continue;
                                    }
                                } else {
                                    // If we can't parse the room count and min_rooms is configured, filter it out
                                    tracing::debug!("Skipping listing #{} - could not determine room count (min required: {}): {}",
                                        index + 1, self.min_rooms, title);
                                    filtered_by_rooms += 1;
                                    continue;
                                }
                            }

                            // Extract surface from title
                            let surface = Self::parse_surface(&title);

                            // Extract price
                            let price_text = Self::extract_price_text(&element);
                            tracing::trace!("Price text extracted: '{}'", price_text);
                            let price = Self::parse_price(&price_text);

                            // Extract image URL
                            let image_url = Self::extract_image_url(&element);

                            // Extract URL
                            let relative_url = Self::extract_relative_url(&element);
                            let full_url = Self::build_full_url(&relative_url);

                            // Extract ID from URL if possible
                            let fallback_id = format!("leboncoin_{}", index);
                            let id = Self::extract_id_from_url(&full_url, &fallback_id);

                            if !title.is_empty() || !full_url.is_empty() {
                                tracing::trace!("Found listing: {} - {} (price: {:?}, surface: {:?}, posted: {})",
                                    id, title, price, surface, posted_at);
                                listings.push(Listing {
                                    id: format!("leboncoin_{}", id),
                                    title: title.trim().to_string(),
                                    price,
                                    surface,
                                    location: city.clone(),
                                    url: full_url,
                                    image_url,
                                    description: None,
                                    posted_at,
                                    source: "Leboncoin".to_string(),
                                });
                                listings_for_city += 1;
                            } else {
                                tracing::trace!("Skipping listing #{} - no title or URL", index);
                            }
                        }

                        tracing::info!("Found {} listings for {} from Leboncoin (filtered {} by age, {} by rooms)",
                            listings_for_city, city, filtered_by_age, filtered_by_rooms);
                    } else {
                        tracing::warn!("No listing elements found for {}. Page structure may have changed.", city);
                        tracing::debug!("HTML preview (first 500 chars): {}",
                            &html.chars().take(500).collect::<String>());
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch listings for {} from Leboncoin: {}", city, e);
                }
            }

            // Be nice to the server - use configured delay
            tokio::time::sleep(tokio::time::Duration::from_millis(self.request_delay_ms)).await;
        }

        Ok(listings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_search_url() {
        let scraper = LeboncoinScraper::new();

        let url = scraper.build_search_url("Paris");
        assert!(url.contains("category=10"));
        assert!(url.contains("PARIS_75000"));
        assert!(url.contains("real_estate_type=2"));
        assert!(url.contains("sort=time"));
        assert!(url.contains("order=desc"));

        let url = scraper.build_search_url("Lyon");
        assert!(url.contains("LYON_69000"));
    }

    #[test]
    fn test_build_search_url_with_spaces() {
        let scraper = LeboncoinScraper::new();

        let url = scraper.build_search_url("Paris 15ème");
        assert!(url.contains("Paris"));
        assert!(url.contains("15"));
        // URL encoding should handle special characters
        assert!(!url.contains(" "));
    }

    #[test]
    fn test_parse_price_standard_format() {
        let price = LeboncoinScraper::parse_price("850 €");
        assert_eq!(price, Some(850.0));
    }

    #[test]
    fn test_parse_price_with_thousands_separator() {
        let price = LeboncoinScraper::parse_price("1 500 €");
        assert_eq!(price, Some(1500.0));
    }

    #[test]
    fn test_parse_price_with_decimals() {
        let price = LeboncoinScraper::parse_price("850,50 €");
        assert_eq!(price, Some(850.50));
    }

    #[test]
    fn test_parse_price_with_non_breaking_space() {
        let price = LeboncoinScraper::parse_price("1\u{00a0}200\u{00a0}€");
        assert_eq!(price, Some(1200.0));
    }

    #[test]
    fn test_parse_price_empty_string() {
        let price = LeboncoinScraper::parse_price("");
        assert_eq!(price, None);
    }

    #[test]
    fn test_parse_price_no_euro_symbol() {
        let price = LeboncoinScraper::parse_price("1250");
        assert_eq!(price, Some(1250.0));
    }

    #[test]
    fn test_parse_surface_from_title_meters_squared() {
        let surface = LeboncoinScraper::parse_surface("Appartement 28 mètres carrés Paris");
        assert_eq!(surface, Some(28.0));
    }

    #[test]
    fn test_parse_surface_from_title_m2_symbol() {
        let surface = LeboncoinScraper::parse_surface("Studio 15m² proche métro");
        assert_eq!(surface, Some(15.0));
    }

    #[test]
    fn test_parse_surface_from_title_with_spaces() {
        let surface = LeboncoinScraper::parse_surface("Colocation 50 m² Bordeaux");
        assert_eq!(surface, Some(50.0));
    }

    #[test]
    fn test_parse_surface_no_match() {
        let surface = LeboncoinScraper::parse_surface("Appartement Paris 15ème");
        assert_eq!(surface, None);
    }

    #[test]
    fn test_parse_surface_empty() {
        let surface = LeboncoinScraper::parse_surface("");
        assert_eq!(surface, None);
    }

    #[test]
    fn test_parse_rooms_t2_format() {
        let rooms = LeboncoinScraper::parse_rooms("Appartement T2 Paris");
        assert_eq!(rooms, Some(2));
    }

    #[test]
    fn test_parse_rooms_t3_format() {
        let rooms = LeboncoinScraper::parse_rooms("Location T3 meublé");
        assert_eq!(rooms, Some(3));
    }

    #[test]
    fn test_parse_rooms_f2_format() {
        let rooms = LeboncoinScraper::parse_rooms("F2 proche centre-ville");
        assert_eq!(rooms, Some(2));
    }

    #[test]
    fn test_parse_rooms_pieces_format() {
        let rooms = LeboncoinScraper::parse_rooms("Appartement 3 pièces Lyon");
        assert_eq!(rooms, Some(3));
    }

    #[test]
    fn test_parse_rooms_pieces_with_accent() {
        let rooms = LeboncoinScraper::parse_rooms("Bel appartement 4 pièces");
        assert_eq!(rooms, Some(4));
    }

    #[test]
    fn test_parse_rooms_one_piece() {
        let rooms = LeboncoinScraper::parse_rooms("Appartement, 1 pièce, 15 mètres carrés.");
        assert_eq!(rooms, Some(1));
    }

    #[test]
    fn test_parse_rooms_two_pieces() {
        let rooms = LeboncoinScraper::parse_rooms("Appartement, 2 pièces, 30 mètres carrés.");
        assert_eq!(rooms, Some(2));
    }

    #[test]
    fn test_parse_rooms_chambres_format() {
        let rooms = LeboncoinScraper::parse_rooms("Colocation 2 chambres disponibles");
        // 2 chambres = 2 bedrooms + 1 living room = 3 rooms total
        assert_eq!(rooms, Some(3));
    }

    #[test]
    fn test_parse_rooms_chambre_singular() {
        let rooms = LeboncoinScraper::parse_rooms("Studio avec 1 chambre");
        // 1 chambre = 1 bedroom + 1 living room = 2 rooms total
        assert_eq!(rooms, Some(2));
    }

    #[test]
    fn test_parse_rooms_t1_studio() {
        let rooms = LeboncoinScraper::parse_rooms("Studio T1 meublé");
        assert_eq!(rooms, Some(1));
    }

    #[test]
    fn test_parse_rooms_no_match() {
        let rooms = LeboncoinScraper::parse_rooms("Appartement centre ville");
        assert_eq!(rooms, None);
    }

    #[test]
    fn test_parse_rooms_empty() {
        let rooms = LeboncoinScraper::parse_rooms("");
        assert_eq!(rooms, None);
    }

    #[test]
    fn test_parse_rooms_mixed_format() {
        // Should match the first pattern (T3)
        let rooms = LeboncoinScraper::parse_rooms("T3 avec 2 chambres");
        assert_eq!(rooms, Some(3));
    }

    #[test]
    fn test_build_full_url_absolute() {
        let full_url = LeboncoinScraper::build_full_url("https://www.leboncoin.fr/colocations/123456.htm");
        assert_eq!(full_url, "https://www.leboncoin.fr/colocations/123456.htm");
    }

    #[test]
    fn test_build_full_url_relative() {
        let full_url = LeboncoinScraper::build_full_url("/colocations/123456.htm");
        assert_eq!(full_url, "https://www.leboncoin.fr/colocations/123456.htm");
    }

    #[test]
    fn test_build_full_url_empty() {
        let full_url = LeboncoinScraper::build_full_url("");
        assert_eq!(full_url, "");
    }

    #[test]
    fn test_extract_id_from_url() {
        let id = LeboncoinScraper::extract_id_from_url(
            "https://www.leboncoin.fr/colocations/123456.htm",
            "fallback"
        );
        assert_eq!(id, "123456");
    }

    #[test]
    fn test_extract_id_from_url_with_query_params() {
        let id = LeboncoinScraper::extract_id_from_url(
            "https://www.leboncoin.fr/colocations/2345678910.htm?param=value",
            "fallback"
        );
        assert!(id.starts_with("2345678910"));
    }

    #[test]
    fn test_extract_id_from_url_empty() {
        let id = LeboncoinScraper::extract_id_from_url("", "fallback_id");
        assert_eq!(id, "fallback_id");
    }

    #[tokio::test]
    async fn test_scraper_creation() {
        let scraper = LeboncoinScraper::new();
        assert_eq!(scraper.name(), "Leboncoin");
    }

    #[tokio::test]
    async fn test_scraper_with_custom_config() {
        let scraper = LeboncoinScraper::with_config("Custom User Agent", 1000, 1440, 2);
        assert_eq!(scraper.request_delay_ms, 1000);
        assert_eq!(scraper.min_rooms, 2);
    }

    #[test]
    fn test_parse_real_leboncoin_listing_html() {
        // Real-world Leboncoin HTML structure
        let html = r#"
            <html>
                <body>
                    <article data-qa-id="aditem">
                        <a href="/colocations/2456789123.htm">
                            <div>
                                <p data-qa-id="aditem_title">Colocation 25m² Lyon 3ème arrondissement</p>
                                <div>
                                    <p data-test-id="price"><span>650 €</span></p>
                                </div>
                                <img src="https://img.leboncoin.fr/api/v1/lbcpb1/images/ab/cd/ef/abcdef123456.jpg" />
                            </div>
                        </a>
                    </article>
                </body>
            </html>
        "#;

        let document = Html::parse_document(html);
        let article_selector = Selector::parse("article[data-qa-id='aditem']").unwrap();
        let element = document.select(&article_selector).next().unwrap();

        // Test using the extraction helper functions
        let title = LeboncoinScraper::extract_title(&element);
        assert_eq!(title, "Colocation 25m² Lyon 3ème arrondissement");

        let surface = LeboncoinScraper::parse_surface(&title);
        assert_eq!(surface, Some(25.0));

        let price_text = LeboncoinScraper::extract_price_text(&element);
        let price = LeboncoinScraper::parse_price(&price_text);
        assert_eq!(price, Some(650.0));

        let relative_url = LeboncoinScraper::extract_relative_url(&element);
        let full_url = LeboncoinScraper::build_full_url(&relative_url);
        assert_eq!(full_url, "https://www.leboncoin.fr/colocations/2456789123.htm");

        let id = LeboncoinScraper::extract_id_from_url(&full_url, "fallback");
        assert_eq!(id, "2456789123");

        let image_url = LeboncoinScraper::extract_image_url(&element);
        assert!(image_url.is_some());
        assert!(image_url.unwrap().contains("leboncoin.fr"));
    }

    #[test]
    fn test_parse_real_leboncoin_listing_with_high_price() {
        // Test parsing of listing with thousands separator
        let html = r#"
            <article data-qa-id="aditem">
                <p data-qa-id="aditem_title">Studio 18 mètres carrés Paris 15ème</p>
                <p data-test-id="price"><span>1 250 €</span></p>
                <a href="/colocations/1234567890.htm"></a>
            </article>
        "#;

        let document = Html::parse_document(html);
        let article_selector = Selector::parse("article[data-qa-id='aditem']").unwrap();
        let element = document.select(&article_selector).next().unwrap();

        let price_text = LeboncoinScraper::extract_price_text(&element);
        let price = LeboncoinScraper::parse_price(&price_text);
        assert_eq!(price, Some(1250.0));

        let title = LeboncoinScraper::extract_title(&element);
        let surface = LeboncoinScraper::parse_surface(&title);
        assert_eq!(surface, Some(18.0));
    }

    #[test]
    fn test_parse_real_leboncoin_listing_no_surface() {
        // Some listings don't have surface information in the title
        let html = r#"
            <article data-qa-id="aditem">
                <p data-qa-id="aditem_title">Chambre meublée proche gare</p>
                <p data-test-id="price"><span>450 €</span></p>
                <a href="/colocations/9876543210.htm"></a>
            </article>
        "#;

        let document = Html::parse_document(html);
        let article_selector = Selector::parse("article[data-qa-id='aditem']").unwrap();
        let element = document.select(&article_selector).next().unwrap();

        let title = LeboncoinScraper::extract_title(&element);
        let surface = LeboncoinScraper::parse_surface(&title);
        assert_eq!(surface, None, "Should return None when no surface info in title");
    }

    #[test]
    fn test_parse_real_leboncoin_multiple_listings() {
        // Test parsing multiple listings at once
        let html = r#"
            <html>
                <body>
                    <article data-qa-id="aditem">
                        <p data-qa-id="aditem_title">Studio 20m² Paris</p>
                        <p data-test-id="price"><span>800 €</span></p>
                        <a href="/colocations/111.htm"></a>
                    </article>
                    <article data-qa-id="aditem">
                        <p data-qa-id="aditem_title">Colocation 30 mètres carrés Lyon</p>
                        <p data-test-id="price"><span>550 €</span></p>
                        <a href="/colocations/222.htm"></a>
                    </article>
                    <article data-qa-id="aditem">
                        <p data-qa-id="aditem_title">Appartement Bordeaux</p>
                        <p data-test-id="price"><span>1 100 €</span></p>
                        <a href="/colocations/333.htm"></a>
                    </article>
                </body>
            </html>
        "#;

        let document = Html::parse_document(html);
        let selector = Selector::parse("article[data-qa-id='aditem']").unwrap();
        let count = document.select(&selector).count();

        assert_eq!(count, 3, "Should find 3 article elements");

        // Verify we can extract data from each using helper functions
        let mut titles = Vec::new();
        let mut prices = Vec::new();

        for element in document.select(&selector) {
            let title = LeboncoinScraper::extract_title(&element);
            titles.push(title);

            let price_text = LeboncoinScraper::extract_price_text(&element);
            let price = LeboncoinScraper::parse_price(&price_text);
            prices.push(price);
        }

        assert_eq!(titles.len(), 3);
        assert_eq!(prices, vec![Some(800.0), Some(550.0), Some(1100.0)]);
    }

    #[test]
    fn test_parse_real_leboncoin_listing_empty_should_skip() {
        // Test that listings with no title and no URL should be skipped
        let html = r#"
            <article data-qa-id="aditem">
                <p data-test-id="price"><span>500 €</span></p>
            </article>
        "#;

        let document = Html::parse_document(html);
        let article_selector = Selector::parse("article[data-qa-id='aditem']").unwrap();
        let element = document.select(&article_selector).next().unwrap();

        let title = LeboncoinScraper::extract_title(&element);
        let relative_url = LeboncoinScraper::extract_relative_url(&element);

        let should_skip = title.is_empty() && relative_url.is_empty();
        assert!(should_skip, "Listing with no title and no URL should be skipped");
    }

    #[test]
    fn test_parse_french_datetime_with_a() {
        use chrono::{Datelike, Timelike};

        // Test parsing "19 février 2026 à 23:00"
        let result = LeboncoinScraper::parse_french_datetime("19 février 2026 à 23:00");
        assert!(result.is_some(), "Should parse French datetime with 'à'");

        if let Some(dt) = result {
            assert_eq!(dt.day(), 19);
            assert_eq!(dt.month(), 2); // février = February
            assert_eq!(dt.year(), 2026);
            assert_eq!(dt.hour(), 22); // 23:00 Paris time (UTC+1) = 22:00 UTC
            assert_eq!(dt.minute(), 0);
        }
    }

    #[test]
    fn test_parse_french_datetime_aujourdhui() {
        // Test parsing "Aujourd'hui, 14:30"
        let result = LeboncoinScraper::parse_french_datetime("Aujourd'hui, 14:30");
        assert!(result.is_some(), "Should parse 'Aujourd'hui' datetime");
    }

    #[test]
    fn test_parse_french_datetime_hier() {
        // Test parsing "Hier, 10:15"
        let result = LeboncoinScraper::parse_french_datetime("Hier, 10:15");
        assert!(result.is_some(), "Should parse 'Hier' datetime");
    }

    #[test]
    fn test_extract_posted_at_from_html() {
        use chrono::Datelike;

        // Test extraction from actual HTML structure
        let html = r#"
            <article data-qa-id="aditem">
                <p data-qa-id="aditem_title">Studio 20m² Paris</p>
                <p title="19 février 2026 à 23:00" class="text-caption text-neutral" aria-hidden="true">Il y a 2 h</p>
                <p data-test-id="price"><span>800 €</span></p>
                <a href="/colocations/111.htm"></a>
            </article>
        "#;

        let document = Html::parse_document(html);
        let article_selector = Selector::parse("article[data-qa-id='aditem']").unwrap();
        let element = document.select(&article_selector).next().unwrap();

        let posted_at = LeboncoinScraper::extract_posted_at(&element);
        assert!(posted_at.is_some(), "Should extract posted_at from p[title] attribute");

        if let Some(dt) = posted_at {
            assert_eq!(dt.day(), 19);
            assert_eq!(dt.month(), 2);
            assert_eq!(dt.year(), 2026);
        }
    }
}

