mod bot;
mod config;
mod database;
mod http_client;
mod models;
mod scraper_trait;
mod scrapers;
mod tracker;

use anyhow::Result;
use bot::{get_intents, send_listing_notification, Bot};
use clap::Parser;
use config::Config;
use database::Database;
use scraper_trait::ScraperRegistry;
use scrapers::LeboncoinScraper;
use serenity::Client;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Parser, Debug)]
#[command(name = "colocfinder")]
#[command(about = "Discord bot for monitoring apartment listings", long_about = None)]
struct Args {
    /// Test URL fetching - fetch and print HTML from a URL
    #[arg(long)]
    test_url: Option<String>,
    
    /// Test a specific scraper with configured cities
    #[arg(long)]
    test_scraper: Option<String>,
    
    /// Save HTML to file when using --test-url
    #[arg(long)]
    save_html: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Handle test-url command
    if let Some(url) = args.test_url {
        return test_url_fetch(&url, args.save_html.as_deref()).await;
    }

    // Load or create config first (before logging is initialized)
    let config = match Config::load() {
        Ok(cfg) => cfg,
        Err(_) => {
            // Use basic logging for this initial message
            eprintln!("No config file found, creating default data/config.yaml");
            Config::create_default()?;
            eprintln!("Please edit data/config.yaml with your Discord token and channel ID");
            return Ok(());
        }
    };

    // Initialize logging - use RUST_LOG env var if set, otherwise use config
    if std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
        tracing::info!("Logging level set from RUST_LOG environment variable");
    } else {
        let level = config.tracing_level.to_lowercase();
        let env_filter = match level.as_str() {
            "trace" => tracing::Level::TRACE,
            "debug" => tracing::Level::DEBUG,
            "info" => tracing::Level::INFO,
            "warn" => tracing::Level::WARN,
            "error" => tracing::Level::ERROR,
            _ => {
                eprintln!("Invalid tracing level '{}', using 'info'", level);
                tracing::Level::INFO
            }
        };

        tracing_subscriber::fmt()
            .with_max_level(env_filter)
            .init();

        tracing::info!("Logging level set to: {} (from data/config.yaml)", level);
    }

    // Handle test-scraper command
    if let Some(scraper_name) = args.test_scraper {
        return test_scraper(&scraper_name, &config).await;
    }

    tracing::info!("Starting Colocfinder Discord Bot...");

    // Validate config
    if config.discord_token == "YOUR_DISCORD_BOT_TOKEN" {
        tracing::error!("Please set your Discord bot token in data/config.yaml");
        return Ok(());
    }

    if config.channel_id == 0 {
        tracing::error!("Please set your Discord channel ID in data/config.yaml");
        return Ok(());
    }

    if config.interesting_channel_id == 0 {
        tracing::error!("Please set your interesting_channel_id in data/config.yaml");
        return Ok(());
    }

    // Initialize scraper registry (Leboncoin only)
    let mut registry = ScraperRegistry::new();
    let leboncoin_scraper = LeboncoinScraper::with_config(
        &config.user_agent,
        config.request_delay_ms,
        config.max_listing_age_minutes,
        config.min_rooms
    );

    // Try to load cookies from file if it exists
    if std::path::Path::new("data/cookies.json").exists() {
        match leboncoin_scraper.load_cookies_from_file("data/cookies.json") {
            Ok(_) => tracing::info!("Successfully loaded cookies from data/cookies.json"),
            Err(e) => tracing::warn!("Failed to load cookies from data/cookies.json: {}", e),
        }
    } else {
        tracing::info!("No data/cookies.json file found. You can export cookies from your browser to avoid captchas.");
        tracing::info!("Use a browser extension like 'EditThisCookie' or 'Cookie Editor' to export cookies as JSON.");
    }

    registry.register(Box::new(leboncoin_scraper));

    tracing::info!("Registered scrapers: {:?}", registry.list_scrapers());
    tracing::info!("Max listing age: {} minutes", config.max_listing_age_minutes);

    // Initialize database
    std::fs::create_dir_all("data")?;
    let db = Arc::new(Mutex::new(Database::new("data/listings.db")?));
    tracing::info!("Database initialized");

    // Setup Discord bot
    let bot = Bot::new();
    bot.set_channel_id(config.channel_id);
    bot.set_interesting_channel_id(config.interesting_channel_id);
    let paused_state = bot.get_paused_state();
    let db_for_bot = db.clone();
    bot.set_database(db_for_bot);

    let intents = get_intents();
    let mut client = Client::builder(&config.discord_token, intents)
        .event_handler(bot)
        .await?;

    let http = client.http.clone();

    // Spawn scraping task
    let registry = Arc::new(registry);
    let config_clone = config.clone();
    let db_clone = db.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(
            tokio::time::Duration::from_secs(config_clone.check_interval_seconds)
        );

        loop {
            interval.tick().await;

            // Check if the bot is paused
            let is_paused = *paused_state.lock().await;
            if is_paused {
                tracing::debug!("Bot is paused, skipping scraping cycle");
                continue;
            }

            tracing::info!("Starting scraping cycle...");

            match registry.scrape_all(&config_clone.cities).await {
                Ok(listings) => {
                    tracing::info!("Found {} total listings", listings.len());

                    // Insert listings into database
                    let db = db_clone.lock().await;
                    let mut new_count = 0;

                    for listing in listings {
                        if !listing.has_sufficient_info() {
                            tracing::debug!("Skipping listing '{}' - insufficient information", listing.title);
                            continue;
                        }

                        match db.insert_or_get_listing(&listing) {
                            Ok(uuid) => {
                                // Check if this listing has been posted yet
                                if let Ok(Some(record)) = db.get_listing_by_uuid(&uuid) {
                                    if record.main_channel_message_id.is_none() {
                                        new_count += 1;
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!("Failed to insert listing into database: {}", e);
                            }
                        }
                    }

                    if new_count == 0 {
                        tracing::info!("No new listings to post");
                    } else {
                        tracing::info!("Found {} new listings to post!", new_count);

                        // Get new listings from database
                        match db.get_new_listings(config_clone.max_listing_age_minutes) {
                            Ok(new_listings) => {
                                drop(db); // Release lock before sending messages

                                // Send notifications
                                for (uuid, listing) in new_listings {
                                    if let Err(e) = send_listing_notification(
                                        &http,
                                        config_clone.channel_id,
                                        &listing,
                                        uuid,
                                        db_clone.clone(),
                                    ).await {
                                        tracing::error!("Failed to send notification: {}", e);
                                    } else {
                                        tracing::info!("Sent notification for: {}", listing.title);
                                    }

                                    // Small delay between messages
                                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                                }
                            }
                            Err(e) => {
                                tracing::error!("Failed to get new listings from database: {}", e);
                            }
                        }

                        // Clean up old unposted listings from database
                        let db = db_clone.lock().await;
                        if let Err(e) = db.cleanup_old_listings(config_clone.max_listing_age_minutes) {
                            tracing::error!("Failed to cleanup old listings: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Scraping failed: {}", e);
                }
            }
        }
    });

    tracing::info!("Bot is starting...");

    // Start the bot
    client.start().await?;

    Ok(())
}

/// Test URL fetching - downloads and prints HTML response
async fn test_url_fetch(url: &str, save_path: Option<&str>) -> Result<()> {
    println!("Testing URL fetch: {}", url);
    println!("{}", "=".repeat(80));
    
    // Try to load config for user agent, otherwise use default
    let user_agent = if let Ok(config) = Config::load() {
        config.user_agent
    } else {
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36".to_string()
    };
    
    println!("User-Agent: {}", user_agent);
    
    // Create HTTP client with cookie jar, same as the bot
    use reqwest::cookie::Jar;
    use std::sync::Arc;

    let cookie_jar = Arc::new(Jar::default());
    let client = http_client::create_http_client_with_cookies(&user_agent, Some(cookie_jar.clone()))?;

    // Try to load cookies from file if it exists
    if std::path::Path::new("data/cookies.json").exists() {
        println!("Loading cookies from data/cookies.json...");
        use std::fs;

        let cookie_data = fs::read_to_string("data/cookies.json")?;
        let cookies: Vec<serde_json::Value> = serde_json::from_str(&cookie_data)?;

        let parsed_url = url.parse::<reqwest::Url>()?;
        let base_url = format!("{}://{}", parsed_url.scheme(), parsed_url.host_str().unwrap_or(""));
        let cookie_url = base_url.parse::<reqwest::Url>()?;

        let mut loaded_count = 0;
        for cookie in &cookies {
            if let (Some(name), Some(value)) = (cookie.get("name"), cookie.get("value")) {
                let name = name.as_str().unwrap_or("");
                let value = value.as_str().unwrap_or("");

                let cookie_str = format!("{}={}", name, value);
                cookie_jar.add_cookie_str(&cookie_str, &cookie_url);
                loaded_count += 1;
            }
        }

        println!("Loaded {} cookies", loaded_count);
    } else {
        println!("No cookies.json found - continuing without cookies");
    }

    println!("Sending request...");
    let response = client.get(url).send().await?;
    
    println!("Status: {}", response.status());
    println!("\nResponse Headers:");
    for (name, value) in response.headers() {
        println!("  {}: {:?}", name, value);
    }
    
    println!("{}", "=".repeat(80));
    
    let body = response.text().await?;
    
    // Save to file if requested
    if let Some(path) = save_path {
        std::fs::write(path, &body)?;
        println!("HTML saved to: {}", path);
        println!("{}", "=".repeat(80));
    } else {
        println!("Response body:");
        println!("{}", "=".repeat(80));
        println!("{}", body);
        println!("{}", "=".repeat(80));
    }
    
    println!("Total length: {} bytes", body.len());
    
    // Check for common CAPTCHA indicators
    let lower_body = body.to_lowercase();
    if lower_body.contains("captcha") || lower_body.contains("recaptcha") || lower_body.contains("cloudflare") {
        println!("\n⚠️  WARNING: Response may contain CAPTCHA or anti-bot protection!");
        println!("Consider:");
        println!("  - Increasing request_delay_ms in config");
        println!("  - Changing user_agent in config");
        println!("  - Using a different IP/proxy");
        println!("  - Checking if the site requires cookies/session");
    }
    
    Ok(())
}

/// Test a specific scraper
async fn test_scraper(scraper_name: &str, config: &Config) -> Result<()> {
    println!("Testing scraper: {}", scraper_name);
    println!("Cities: {:?}", config.cities);
    println!("User-Agent: {}", config.user_agent);
    println!("Request delay: {}ms", config.request_delay_ms);
    println!("{}", "=".repeat(80));
    
    let scraper: Box<dyn scraper_trait::Scraper> = match scraper_name.to_lowercase().as_str() {
        "leboncoin" => {
            let leboncoin_scraper = LeboncoinScraper::with_config(
                &config.user_agent,
                config.request_delay_ms,
                config.max_listing_age_minutes,
                config.min_rooms
            );

            // Try to load cookies from file if it exists (same as the bot)
            if std::path::Path::new("cookies.json").exists() {
                println!("Loading cookies from cookies.json...");
                match leboncoin_scraper.load_cookies_from_file("cookies.json") {
                    Ok(_) => println!("✓ Successfully loaded cookies from cookies.json"),
                    Err(e) => println!("⚠ Failed to load cookies from cookies.json: {}", e),
                }
            } else {
                println!("No cookies.json file found. You can export cookies from your browser to avoid captchas.");
            }

            Box::new(leboncoin_scraper)
        }
        name => {
            eprintln!("Unknown scraper: {}", name);
            eprintln!("Available scrapers: leboncoin");
            return Ok(());
        }
    };

    println!("Running scraper...");
    match scraper.scrape(&config.cities).await {
        Ok(listings) => {
            println!("Found {} listings", listings.len());
            println!("{}", "=".repeat(80));

            for (i, listing) in listings.iter().enumerate() {
                println!("\nListing #{}", i + 1);
                println!("ID: {}", listing.id);
                println!("Title: {}", listing.title);
                println!("Price: {:?}", listing.price);
                println!("Surface: {:?}", listing.surface);
                println!("Location: {}", listing.location);
                println!("URL: {}", listing.url);
                println!("Image: {:?}", listing.image_url);
                if let Some(desc) = &listing.description {
                    println!("Description: {}", desc);
                }
                println!("Source: {}", listing.source);
                println!("{}", "-".repeat(80));
            }

            if listings.is_empty() {
                println!("No listings found. This might mean:");
                println!("  - The scraper selectors need updating");
                println!("  - The website structure has changed");
                println!("  - No listings match the search criteria");
            }
        }
        Err(e) => {
            eprintln!("Error scraping: {}", e);
        }
    }

    Ok(())
}

