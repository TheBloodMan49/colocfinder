use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;
use crate::models::Listing;

#[derive(Debug, Clone, PartialEq)]
pub enum ListingStatus {
    Unchecked,
    Interesting,
    Verified,
    NotGood,
}

impl ListingStatus {
    fn to_string(&self) -> &str {
        match self {
            ListingStatus::Unchecked => "unchecked",
            ListingStatus::Interesting => "interesting",
            ListingStatus::Verified => "verified",
            ListingStatus::NotGood => "not_good",
        }
    }

    fn from_string(s: &str) -> Self {
        match s {
            "interesting" => ListingStatus::Interesting,
            "verified" => ListingStatus::Verified,
            "not_good" => ListingStatus::NotGood,
            _ => ListingStatus::Unchecked,
        }
    }
}

pub struct ListingRecord {
    pub uuid: Uuid,
    pub listing_id: String,
    pub title: String,
    pub price: Option<f64>,
    pub surface: Option<f64>,
    pub location: String,
    pub url: String,
    pub image_url: Option<String>,
    pub description: Option<String>,
    pub posted_at: DateTime<Utc>,
    pub source: String,
    pub status: ListingStatus,
    pub scraped_at: DateTime<Utc>,
    pub main_channel_message_id: Option<u64>,
    pub interesting_channel_message_id: Option<u64>,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Database { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS listings (
                uuid TEXT PRIMARY KEY,
                listing_id TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                price REAL,
                surface REAL,
                location TEXT NOT NULL,
                url TEXT NOT NULL,
                image_url TEXT,
                description TEXT,
                posted_at TEXT,
                source TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'unchecked',
                scraped_at TEXT NOT NULL,
                main_channel_message_id INTEGER,
                interesting_channel_message_id INTEGER
            )",
            [],
        )?;

        // Create index on listing_id for faster lookups
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_listing_id ON listings(listing_id)",
            [],
        )?;

        // Create index on status for filtering
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_status ON listings(status)",
            [],
        )?;

        Ok(())
    }

    /// Insert a new listing or get existing one if already exists
    pub fn insert_or_get_listing(&self, listing: &Listing) -> Result<Uuid> {
        // Check if listing already exists
        if let Some(uuid) = self.get_listing_uuid_by_id(&listing.id)? {
            return Ok(uuid);
        }

        // Generate new UUID and insert
        let uuid = Uuid::new_v4();
        let scraped_at = Utc::now();

        self.conn.execute(
            "INSERT INTO listings (
                uuid, listing_id, title, price, surface, location, url,
                image_url, description, posted_at, source, status, scraped_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                uuid.to_string(),
                &listing.id,
                &listing.title,
                listing.price,
                listing.surface,
                &listing.location,
                &listing.url,
                &listing.image_url,
                &listing.description,
                listing.posted_at,
                &listing.source,
                ListingStatus::Unchecked.to_string(),
                scraped_at,
            ],
        )?;

        Ok(uuid)
    }

    /// Check if a listing exists by its listing ID
    pub fn listing_exists(&self, listing_id: &str) -> Result<bool> {
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM listings WHERE listing_id = ?1)",
            params![listing_id],
            |row| row.get(0),
        )?;
        Ok(exists)
    }

    /// Get UUID for a listing by its listing ID
    pub fn get_listing_uuid_by_id(&self, listing_id: &str) -> Result<Option<Uuid>> {
        let uuid_str: Option<String> = self.conn
            .query_row(
                "SELECT uuid FROM listings WHERE listing_id = ?1",
                params![listing_id],
                |row| row.get(0),
            )
            .optional()?;

        Ok(uuid_str.map(|s| Uuid::parse_str(&s).unwrap()))
    }

    /// Get a listing record by UUID
    pub fn get_listing_by_uuid(&self, uuid: &Uuid) -> Result<Option<ListingRecord>> {
        let record = self.conn
            .query_row(
                "SELECT uuid, listing_id, title, price, surface, location, url,
                        image_url, description, posted_at, source, status, scraped_at,
                        main_channel_message_id, interesting_channel_message_id
                 FROM listings WHERE uuid = ?1",
                params![uuid.to_string()],
                |row| {
                    Ok(ListingRecord {
                        uuid: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                        listing_id: row.get(1)?,
                        title: row.get(2)?,
                        price: row.get(3)?,
                        surface: row.get(4)?,
                        location: row.get(5)?,
                        url: row.get(6)?,
                        image_url: row.get(7)?,
                        description: row.get(8)?,
                        posted_at: row.get(9)?,
                        source: row.get(10)?,
                        status: ListingStatus::from_string(&row.get::<_, String>(11)?),
                        scraped_at: row.get(12)?,
                        main_channel_message_id: row.get(13)?,
                        interesting_channel_message_id: row.get(14)?,
                    })
                },
            )
            .optional()?;

        Ok(record)
    }

    /// Update the status of a listing
    pub fn update_status(&self, uuid: &Uuid, status: ListingStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE listings SET status = ?1 WHERE uuid = ?2",
            params![status.to_string(), uuid.to_string()],
        )?;
        Ok(())
    }

    /// Set the main channel message ID for a listing
    pub fn set_main_channel_message_id(&self, uuid: &Uuid, message_id: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE listings SET main_channel_message_id = ?1 WHERE uuid = ?2",
            params![message_id, uuid.to_string()],
        )?;
        Ok(())
    }

    /// Set the interesting channel message ID for a listing
    pub fn set_interesting_channel_message_id(&self, uuid: &Uuid, message_id: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE listings SET interesting_channel_message_id = ?1 WHERE uuid = ?2",
            params![message_id, uuid.to_string()],
        )?;
        Ok(())
    }

    /// Clear the interesting channel message ID for a listing (when removed from interesting)
    pub fn clear_interesting_channel_message_id(&self, uuid: &Uuid) -> Result<()> {
        self.conn.execute(
            "UPDATE listings SET interesting_channel_message_id = NULL WHERE uuid = ?1",
            params![uuid.to_string()],
        )?;
        Ok(())
    }

    /// Get all new listings (unchecked status, no main channel message)
    /// Filters out listings older than max_listing_age_minutes
    pub fn get_new_listings(&self, max_listing_age_minutes: u64) -> Result<Vec<(Uuid, Listing)>> {
        let mut stmt = self.conn.prepare(
            "SELECT uuid, listing_id, title, price, surface, location, url,
                    image_url, description, posted_at, source
             FROM listings
             WHERE main_channel_message_id IS NULL
             ORDER BY scraped_at DESC"
        )?;

        let now = Utc::now();
        let max_age = chrono::Duration::minutes(max_listing_age_minutes as i64);

        let listings = stmt
            .query_map([], |row| {
                let uuid = Uuid::parse_str(&row.get::<_, String>(0)?).unwrap();
                let listing = Listing {
                    id: row.get(1)?,
                    title: row.get(2)?,
                    price: row.get(3)?,
                    surface: row.get(4)?,
                    location: row.get(5)?,
                    url: row.get(6)?,
                    image_url: row.get(7)?,
                    description: row.get(8)?,
                    posted_at: row.get(9)?,
                    source: row.get(10)?,
                };
                Ok((uuid, listing))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|(_, listing)| {
                // Check if the listing is too old
                let age = now.signed_duration_since(listing.posted_at);
                if age > max_age {
                    tracing::debug!(
                        "Filtering out old listing '{}' - age: {} minutes (max: {})",
                        listing.title, age.num_minutes(), max_listing_age_minutes
                    );
                    return false;
                }
                true
            })
            .collect();

        Ok(listings)
    }

    /// Delete old unposted listings that are past the max age
    /// This helps keep the database clean by removing stale listings that were never posted
    pub fn cleanup_old_listings(&self, max_listing_age_minutes: u64) -> Result<usize> {
        let now = Utc::now();
        let cutoff_time = now - chrono::Duration::minutes(max_listing_age_minutes as i64);

        let deleted = self.conn.execute(
            "DELETE FROM listings
             WHERE main_channel_message_id IS NULL
             AND posted_at IS NOT NULL
             AND posted_at < ?1",
            params![cutoff_time],
        )?;

        if deleted > 0 {
            tracing::info!("Cleaned up {} old unposted listings from database", deleted);
        }

        Ok(deleted)
    }
}
