use serenity::all::{
    ChannelId, Command, Context, CreateCommand, CreateInteractionResponse,
    CreateInteractionResponseMessage, CreateMessage, EventHandler, GatewayIntents,
    Interaction, Ready, Http, CreateEmbed, Colour, Timestamp, CreateButton, CreateActionRow,
    ButtonStyle, ReactionType, Reaction, EditMessage, ComponentInteraction,
};
use serenity::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;
use crate::database::{Database, ListingStatus};
use crate::models::Listing;

pub struct Bot {
    channel_id: Arc<Mutex<Option<u64>>>,
    interesting_channel_id: Arc<Mutex<Option<u64>>>,
    paused: Arc<Mutex<bool>>,
    database: Arc<Mutex<Option<Arc<Mutex<Database>>>>>,
}

impl Bot {
    pub fn new() -> Self {
        Self {
            channel_id: Arc::new(Mutex::new(None)),
            interesting_channel_id: Arc::new(Mutex::new(None)),
            paused: Arc::new(Mutex::new(false)),
            database: Arc::new(Mutex::new(None)),
        }
    }

    pub fn set_channel_id(&self, channel_id: u64) {
        let channel_id_clone = self.channel_id.clone();
        tokio::spawn(async move {
            let mut id = channel_id_clone.lock().await;
            *id = Some(channel_id);
        });
    }

    pub fn set_interesting_channel_id(&self, channel_id: u64) {
        let interesting_channel_id_clone = self.interesting_channel_id.clone();
        tokio::spawn(async move {
            let mut id = interesting_channel_id_clone.lock().await;
            *id = Some(channel_id);
        });
    }

    pub fn set_database(&self, database: Arc<Mutex<Database>>) {
        let database_clone = self.database.clone();
        tokio::spawn(async move {
            let mut db = database_clone.lock().await;
            *db = Some(database);
        });
    }

    pub fn get_paused_state(&self) -> Arc<Mutex<bool>> {
        self.paused.clone()
    }

    pub fn get_interesting_channel_id(&self) -> Arc<Mutex<Option<u64>>> {
        self.interesting_channel_id.clone()
    }

    pub fn get_database(&self) -> Arc<Mutex<Option<Arc<Mutex<Database>>>>> {
        self.database.clone()
    }
}

#[async_trait]
impl EventHandler for Bot {
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        match interaction {
            Interaction::Command(command) => {
                let response = match command.data.name.as_str() {
                    "ping" => {
                        CreateInteractionResponse::Message(
                            CreateInteractionResponseMessage::new()
                                .content("Pong! 🏓")
                        )
                    }
                    "status" => {
                        let paused = *self.paused.lock().await;
                        let status_msg = if paused {
                            "⏸️ Bot is **paused**. Use `/resume` to continue monitoring."
                        } else {
                            "✅ Bot is **running** and monitoring for new listings!"
                        };
                        CreateInteractionResponse::Message(
                            CreateInteractionResponseMessage::new()
                                .content(status_msg)
                        )
                    }
                    "pause" => {
                        let mut paused = self.paused.lock().await;
                        if *paused {
                            CreateInteractionResponse::Message(
                                CreateInteractionResponseMessage::new()
                                    .content("ℹ️ Bot is already paused.")
                            )
                        } else {
                            *paused = true;
                            CreateInteractionResponse::Message(
                                CreateInteractionResponseMessage::new()
                                    .content("⏸️ Bot monitoring **paused**. Use `/resume` to continue.")
                            )
                        }
                    }
                    "resume" => {
                        let mut paused = self.paused.lock().await;
                        if !*paused {
                            CreateInteractionResponse::Message(
                                CreateInteractionResponseMessage::new()
                                    .content("ℹ️ Bot is already running.")
                            )
                        } else {
                            *paused = false;
                            CreateInteractionResponse::Message(
                                CreateInteractionResponseMessage::new()
                                    .content("▶️ Bot monitoring **resumed**. Watching for new listings!")
                            )
                        }
                    }
                    "clear" => {
                        // Acknowledge first with ephemeral message
                        if let Err(e) = command.create_response(&ctx.http,
                            CreateInteractionResponse::Message(
                                CreateInteractionResponseMessage::new()
                                    .content("🗑️ Clearing bot messages from this channel...")
                                    .ephemeral(true)
                            )
                        ).await {
                            tracing::error!("Error acknowledging clear command: {:?}", e);
                            return;
                        }

                        // Get the channel ID from the command
                        let channel_id = command.channel_id;

                        // Clear messages asynchronously
                        let ctx_clone = ctx.clone();
                        tokio::spawn(async move {
                            match clear_bot_messages(&ctx_clone, channel_id).await {
                                Ok(count) => {
                                    tracing::info!("Cleared {} bot messages from channel {}", count, channel_id);
                                    // Try to edit the response to show completion
                                    let _ = command.edit_response(&ctx_clone.http,
                                        serenity::all::EditInteractionResponse::new()
                                            .content(format!("✅ Cleared {} bot message(s) from this channel!", count))
                                    ).await;
                                }
                                Err(e) => {
                                    tracing::error!("Error clearing messages: {:?}", e);
                                    let _ = command.edit_response(&ctx_clone.http,
                                        serenity::all::EditInteractionResponse::new()
                                            .content(format!("❌ Error clearing messages: {}", e))
                                    ).await;
                                }
                            }
                        });
                        return; // Early return since we already responded
                    }
                    _ => {
                        CreateInteractionResponse::Message(
                            CreateInteractionResponseMessage::new()
                                .content("Unknown command")
                        )
                    }
                };

                if let Err(e) = command.create_response(&ctx.http, response).await {
                    tracing::error!("Error responding to command: {:?}", e);
                }
            }
            Interaction::Component(component) => {
                // Handle button interactions
                let db_option = self.database.lock().await.clone();
                if let Some(db) = db_option {
                    if component.data.custom_id == "interesting_listing" {
                        let interesting_channel_id = self.interesting_channel_id.lock().await.clone();
                        if let Err(e) = handle_interesting_button(&ctx, &component, interesting_channel_id, db.clone()).await {
                            tracing::error!("Error handling interesting button: {:?}", e);
                        }
                    } else if component.data.custom_id == "remove_from_interesting" {
                        let main_channel_id = self.channel_id.lock().await.clone();
                        if let Err(e) = handle_remove_from_interesting_button(&ctx, &component, db.clone(), main_channel_id).await {
                            tracing::error!("Error handling remove from interesting button: {:?}", e);
                        }
                    } else if component.data.custom_id == "not_good_listing" {
                        if let Err(e) = handle_not_good_button(&ctx, &component, db.clone()).await {
                            tracing::error!("Error handling not good button: {:?}", e);
                        }
                    }
                } else {
                    tracing::error!("Database not initialized");
                }
            }
            _ => {}
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        tracing::info!("Discord bot {} is connected!", ready.user.name);

        // Register slash commands
        let commands = vec![
            CreateCommand::new("ping").description("Check if the bot is responsive"),
            CreateCommand::new("status").description("Get the current status of the bot"),
            CreateCommand::new("pause").description("Pause the listing monitoring"),
            CreateCommand::new("resume").description("Resume the listing monitoring"),
            CreateCommand::new("clear").description("Remove all bot messages from the current channel"),
        ];

        if let Err(e) = Command::set_global_commands(&ctx.http, commands).await {
            tracing::error!("Failed to register commands: {:?}", e);
        } else {
            tracing::info!("Successfully registered slash commands");
        }
    }

    async fn reaction_add(&self, ctx: Context, reaction: Reaction) {
        // Check if it's a red X emoji (❌)
        if let ReactionType::Unicode(emoji) = &reaction.emoji {
            if emoji == "❌" {
                // Only handle red X in the main channel, not the interesting channel
                let main_channel_id = self.channel_id.lock().await.clone();
                if let Some(main_id) = main_channel_id {
                    if reaction.channel_id.get() == main_id {
                        let db_option = self.database.lock().await.clone();
                        if let Some(db) = db_option {
                            if let Err(e) = handle_red_x_reaction(&ctx, &reaction, db).await {
                                tracing::error!("Error handling red X reaction: {:?}", e);
                            }
                        }
                    } else {
                        tracing::debug!("Ignoring red X reaction in non-main channel");
                    }
                }
            }
        }
    }
}

pub async fn send_listing_notification(
    http: &Arc<Http>,
    channel_id: u64,
    listing: &Listing,
    uuid: Uuid,
    database: Arc<Mutex<Database>>,
) -> Result<(), serenity::Error> {
    // Check if this listing already has a message on Discord
    {
        let db = database.lock().await;
        if let Ok(Some(record)) = db.get_listing_by_uuid(&uuid) {
            if record.main_channel_message_id.is_some() {
                tracing::warn!("Listing '{}' already has a Discord message (ID: {:?}), skipping",
                    listing.title, record.main_channel_message_id);
                return Ok(());
            }
        }
    }

    // Skip listings without sufficient information
    if !listing.has_sufficient_info() {
        tracing::warn!("Skipping listing '{}' - insufficient information", listing.title);
        // Mark as attempted with a special message ID (0) to prevent retrying
        let db = database.lock().await;
        if let Err(e) = db.set_main_channel_message_id(&uuid, 0) {
            tracing::error!("Failed to mark listing as skipped: {}", e);
        }
        return Ok(());
    }

    let channel = ChannelId::new(channel_id);

    // Create embed with listing image (dark red color for unverified)
    let mut embed = CreateEmbed::new()
        .title(&listing.title)
        .url(&listing.url)
        .color(Colour::from_rgb(139, 0, 0)); // Dark red color for unverified listings

    // Add image if available
    if let Some(image_url) = &listing.image_url {
        tracing::debug!("Adding image to embed: {}", image_url);
        embed = embed.image(image_url);
    } else {
        tracing::debug!("No image URL available for listing");
    }

    // Add price if available (prominently)
    if let Some(price) = listing.price {
        embed = embed.field("💰 Prix", format!("**{:.0}€**", price), true);
    }

    // Add surface if available
    if let Some(surface) = listing.surface {
        embed = embed.field("📐 Surface", format!("**{:.0}m²**", surface), true);
    }

    // Add posted time as both relative and absolute time
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(listing.posted_at);

    let time_str = if duration.num_minutes() < 1 {
        "À l'instant".to_string()
    } else if duration.num_minutes() < 60 {
        format!("Il y a {} min", duration.num_minutes())
    } else if duration.num_hours() < 24 {
        format!("Il y a {} h", duration.num_hours())
    } else {
        format!("Il y a {} j", duration.num_days())
    };

    // Format the absolute time in Paris timezone
    let formatted_time = listing.posted_at.format("%d/%m/%Y à %H:%M").to_string();
    let combined_time = format!("{}\n({})", time_str, formatted_time);

    embed = embed.field("🕐 Publié", combined_time, true);

    // Add location
    //embed = embed.field("📍 Location", &listing.location, true);

    // Add description if available
    if let Some(desc) = &listing.description {
        let truncated = if desc.len() > 300 {
            format!("{}...", &desc[..300])
        } else {
            desc.clone()
        };
        embed = embed.description(truncated);
    }

    // Add timestamp
    embed = embed.timestamp(Timestamp::from_unix_timestamp(listing.posted_at.timestamp()).unwrap_or_else(|_| Timestamp::now()));

    // Add footer with source and UUID
    embed = embed.footer(serenity::all::CreateEmbedFooter::new(format!("Source: {} | ID: {}", listing.source, uuid)));

    tracing::info!("Sending embed for listing: {} (has image: {}) with UUID: {}",
        listing.title,
        listing.image_url.is_some(),
        uuid
    );

    // Create the "Intéressant" and "Pas bien" buttons for main channel
    let interesting_button = CreateButton::new("interesting_listing")
        .label("Intéressant")
        .style(ButtonStyle::Primary);

    let not_good_button = CreateButton::new("not_good_listing")
        .label("Pas bien")
        .style(ButtonStyle::Danger);

    let action_row = CreateActionRow::Buttons(vec![interesting_button, not_good_button]);

    let builder = CreateMessage::new()
        .embed(embed)
        .components(vec![action_row]);

    let message = channel.send_message(http, builder).await?;

    // Store the message ID in the database
    let db = database.lock().await;
    if let Err(e) = db.set_main_channel_message_id(&uuid, message.id.get()) {
        tracing::error!("Failed to store main channel message ID: {}", e);
    }

    Ok(())
}


fn extract_uuid_from_footer(footer_text: &str) -> Option<Uuid> {
    // Footer format: "Source: leboncoin | ID: uuid"
    if let Some(id_part) = footer_text.split(" | ID: ").nth(1) {
        Uuid::parse_str(id_part.trim()).ok()
    } else {
        None
    }
}

async fn handle_red_x_reaction(ctx: &Context, reaction: &Reaction, database: Arc<Mutex<Database>>) -> Result<(), serenity::Error> {
    // Get the message
    let mut message = reaction.message(&ctx.http).await?;

    // Get the first embed
    if let Some(embed) = message.embeds.first() {
        // Extract UUID from footer to fetch original listing data if needed
        let uuid = if let Some(footer) = &embed.footer {
            extract_uuid_from_footer(&footer.text)
        } else {
            None
        };

        // If we have a UUID and no image in the current embed, restore from database
        let mut image_url_to_restore: Option<String> = None;
        if let Some(uuid) = uuid {
            if embed.image.is_none() {
                // Image was removed (likely by "not good" button), restore from database
                let db = database.lock().await;
                if let Ok(Some(record)) = db.get_listing_by_uuid(&uuid) {
                    image_url_to_restore = record.image_url;
                    tracing::info!("Restoring image from database for UUID: {}", uuid);
                }
                // Update status back to unchecked
                if let Err(e) = db.update_status(&uuid, ListingStatus::Unchecked) {
                    tracing::error!("Failed to update listing status: {}", e);
                }
            }
        }

        // Create a new embed with dark red color
        let mut new_embed = CreateEmbed::new()
            .color(Colour::from_rgb(139, 0, 0)); // Dark red color for unverified

        // Copy all fields from the original embed
        if let Some(title) = &embed.title {
            new_embed = new_embed.title(title);
        }
        if let Some(url) = &embed.url {
            new_embed = new_embed.url(url);
        }
        if let Some(description) = &embed.description {
            new_embed = new_embed.description(description);
        }

        // Restore image: use existing if present, otherwise use restored from database
        if let Some(image) = &embed.image {
            new_embed = new_embed.image(&image.url);
        } else if let Some(restored_image) = image_url_to_restore {
            new_embed = new_embed.image(restored_image);
        }

        if let Some(footer) = &embed.footer {
            new_embed = new_embed.footer(serenity::all::CreateEmbedFooter::new(&footer.text));
        }
        if let Some(timestamp) = &embed.timestamp {
            new_embed = new_embed.timestamp(timestamp.clone());
        }

        // Copy fields
        for field in &embed.fields {
            new_embed = new_embed.field(&field.name, &field.value, field.inline);
        }

        // Recreate the two buttons
        let interesting_button = CreateButton::new("interesting_listing")
            .label("Intéressant")
            .style(ButtonStyle::Primary);

        let not_good_button = CreateButton::new("not_good_listing")
            .label("Pas bien")
            .style(ButtonStyle::Danger);

        let action_row = CreateActionRow::Buttons(vec![interesting_button, not_good_button]);

        // Update the message with the dark red embed and add back the buttons
        let edit = EditMessage::new()
            .embed(new_embed)
            .components(vec![action_row]);

        message.edit(&ctx.http, edit).await?;

        // Remove the reaction
        reaction.delete(&ctx.http).await?;
    }

    Ok(())
}

async fn handle_interesting_button(ctx: &Context, component: &ComponentInteraction, interesting_channel_id: Option<u64>, database: Arc<Mutex<Database>>) -> Result<(), serenity::Error> {
    let message = &component.message;

    if interesting_channel_id.is_none() {
        component.create_response(&ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("❌ Interesting channel is not configured!")
                    .ephemeral(true)
            )
        ).await?;
        return Ok(());
    }

    let interesting_channel = ChannelId::new(interesting_channel_id.unwrap());

    // Get the first embed (our listing embed)
    if let Some(embed) = message.embeds.first() {
        // Extract UUID from footer
        let uuid = if let Some(footer) = &embed.footer {
            extract_uuid_from_footer(&footer.text)
        } else {
            None
        };

        if let Some(uuid) = uuid {
            // Update status in database
            let db = database.lock().await;
            if let Err(e) = db.update_status(&uuid, ListingStatus::Interesting) {
                tracing::error!("Failed to update listing status: {}", e);
            }
        } else {
            tracing::warn!("Could not extract UUID from footer");
        }

        // Create a new embed with the same content for the interesting channel
        let mut new_embed = CreateEmbed::new()
            .color(Colour::from_rgb(139, 0, 0)); // Dark red initially

        // Copy all fields from the original embed
        if let Some(title) = &embed.title {
            new_embed = new_embed.title(title);
        }
        if let Some(url) = &embed.url {
            new_embed = new_embed.url(url);
        }
        if let Some(description) = &embed.description {
            new_embed = new_embed.description(description);
        }
        if let Some(image) = &embed.image {
            new_embed = new_embed.image(&image.url);
        }
        if let Some(footer) = &embed.footer {
            new_embed = new_embed.footer(serenity::all::CreateEmbedFooter::new(&footer.text));
        }
        if let Some(timestamp) = &embed.timestamp {
            new_embed = new_embed.timestamp(timestamp.clone());
        }

        // Copy fields
        for field in &embed.fields {
            new_embed = new_embed.field(&field.name, &field.value, field.inline);
        }

        // Send to interesting channel with only remove button
        let remove_button = CreateButton::new("remove_from_interesting")
            .label("Retirer")
            .style(ButtonStyle::Danger);

        let action_row = CreateActionRow::Buttons(vec![remove_button]);

        let builder = CreateMessage::new()
            .embed(new_embed)
            .components(vec![action_row]);

        let interesting_message = interesting_channel.send_message(&ctx.http, builder).await?;

        // Store the interesting channel message ID in database if we have UUID
        if let Some(uuid) = uuid {
            let db = database.lock().await;
            if let Err(e) = db.set_interesting_channel_message_id(&uuid, interesting_message.id.get()) {
                tracing::error!("Failed to store interesting channel message ID: {}", e);
            }
        }

        // Update the original message to purple color
        let mut purple_embed = CreateEmbed::new()
            .color(Colour::from_rgb(128, 0, 128)); // Purple color for interesting listings

        // Copy all fields from the original embed
        if let Some(title) = &embed.title {
            purple_embed = purple_embed.title(title);
        }
        if let Some(url) = &embed.url {
            purple_embed = purple_embed.url(url);
        }
        if let Some(description) = &embed.description {
            purple_embed = purple_embed.description(description);
        }
        if let Some(image) = &embed.image {
            purple_embed = purple_embed.image(&image.url);
        }
        if let Some(footer) = &embed.footer {
            purple_embed = purple_embed.footer(serenity::all::CreateEmbedFooter::new(&footer.text));
        }
        if let Some(timestamp) = &embed.timestamp {
            purple_embed = purple_embed.timestamp(timestamp.clone());
        }

        // Copy fields
        for field in &embed.fields {
            purple_embed = purple_embed.field(&field.name, &field.value, field.inline);
        }

        // Keep remaining buttons in main channel but disable "Intéressant"
        let interesting_button = CreateButton::new("interesting_listing")
            .label("Intéressant")
            .style(ButtonStyle::Primary)
            .disabled(true); // Disable since already sent

        let not_good_button = CreateButton::new("not_good_listing")
            .label("Pas bien")
            .style(ButtonStyle::Danger);

        let action_row = CreateActionRow::Buttons(vec![interesting_button, not_good_button]);

        let edit = EditMessage::new()
            .embed(purple_embed)
            .components(vec![action_row]);

        message.clone().edit(&ctx.http, edit).await?;
    }

    // Acknowledge the interaction
    component.create_response(&ctx.http,
        CreateInteractionResponse::Acknowledge
    ).await?;

    Ok(())
}

async fn handle_remove_from_interesting_button(ctx: &Context, component: &ComponentInteraction, database: Arc<Mutex<Database>>, main_channel_id: Option<u64>) -> Result<(), serenity::Error> {
    let message = &component.message;

    // Get the first embed (our listing embed)
    if let Some(embed) = message.embeds.first() {
        // Extract UUID from footer
        let uuid = if let Some(footer) = &embed.footer {
            extract_uuid_from_footer(&footer.text)
        } else {
            None
        };

        if let Some(uuid) = uuid {
            // Update status back to unchecked and clear the interesting channel message ID
            let db = database.lock().await;
            if let Err(e) = db.update_status(&uuid, ListingStatus::Unchecked) {
                tracing::error!("Failed to update listing status: {}", e);
            }
            if let Err(e) = db.clear_interesting_channel_message_id(&uuid) {
                tracing::error!("Failed to clear interesting channel message ID: {}", e);
            }

            // Get the main channel message ID to update the original post
            if let Ok(Some(record)) = db.get_listing_by_uuid(&uuid) {
                if let (Some(main_msg_id), Some(channel_id)) = (record.main_channel_message_id, main_channel_id) {
                    drop(db); // Release database lock before Discord API calls

                    // Try to update the main channel message back to dark red
                    let main_channel = ChannelId::new(channel_id);
                    if let Ok(mut main_message) = main_channel.message(&ctx.http, main_msg_id).await {
                        if let Some(main_embed) = main_message.embeds.first() {
                            // Create a new embed with dark red color (back to unchecked)
                            let mut reverted_embed = CreateEmbed::new()
                                .color(Colour::from_rgb(139, 0, 0)); // Dark red for unchecked

                            // Copy all fields from the original embed
                            if let Some(title) = &main_embed.title {
                                reverted_embed = reverted_embed.title(title);
                            }
                            if let Some(url) = &main_embed.url {
                                reverted_embed = reverted_embed.url(url);
                            }
                            if let Some(description) = &main_embed.description {
                                reverted_embed = reverted_embed.description(description);
                            }
                            if let Some(image) = &main_embed.image {
                                reverted_embed = reverted_embed.image(&image.url);
                            }
                            if let Some(footer) = &main_embed.footer {
                                reverted_embed = reverted_embed.footer(serenity::all::CreateEmbedFooter::new(&footer.text));
                            }
                            if let Some(timestamp) = &main_embed.timestamp {
                                reverted_embed = reverted_embed.timestamp(timestamp.clone());
                            }

                            // Copy fields
                            for field in &main_embed.fields {
                                reverted_embed = reverted_embed.field(&field.name, &field.value, field.inline);
                            }

                            // Re-enable all buttons
                            let interesting_button = CreateButton::new("interesting_listing")
                                .label("Intéressant")
                                .style(ButtonStyle::Primary);

                            let not_good_button = CreateButton::new("not_good_listing")
                                .label("Pas bien")
                                .style(ButtonStyle::Danger);

                            let action_row = CreateActionRow::Buttons(vec![interesting_button, not_good_button]);

                            // Update the main channel message
                            let edit = EditMessage::new()
                                .embed(reverted_embed)
                                .components(vec![action_row]);

                            if let Err(e) = main_message.edit(&ctx.http, edit).await {
                                tracing::error!("Failed to update main channel message: {}", e);
                            } else {
                                tracing::info!("Reverted main channel message {} back to unchecked", main_msg_id);
                            }
                        }
                    }
                }
            }
        } else {
            tracing::warn!("Could not extract UUID from footer");
        }

        // Delete the message from the interesting channel
        message.delete(&ctx.http).await?;

        // Acknowledge the interaction
        component.create_response(&ctx.http,
            CreateInteractionResponse::Acknowledge
        ).await?;
    } else {
        // Acknowledge with error if no embed found
        component.create_response(&ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("❌ Could not find listing information!")
                    .ephemeral(true)
            )
        ).await?;
    }

    Ok(())
}

async fn handle_not_good_button(ctx: &Context, component: &ComponentInteraction, database: Arc<Mutex<Database>>) -> Result<(), serenity::Error> {
    // Get the message that was interacted with
    let message = &component.message;

    // Get the first embed (our listing embed)
    if let Some(embed) = message.embeds.first() {
        // Extract UUID from footer
        let uuid = if let Some(footer) = &embed.footer {
            extract_uuid_from_footer(&footer.text)
        } else {
            None
        };

        if let Some(uuid) = uuid {
            // Update status in database
            let db = database.lock().await;
            if let Err(e) = db.update_status(&uuid, ListingStatus::NotGood) {
                tracing::error!("Failed to update listing status: {}", e);
            }
        } else {
            tracing::warn!("Could not extract UUID from footer");
        }

        // Create a new embed with black color and NO image
        let mut new_embed = CreateEmbed::new()
            .color(Colour::from_rgb(0, 0, 0)); // Black color for not good listings

        // Copy all fields from the original embed EXCEPT the image
        if let Some(title) = &embed.title {
            new_embed = new_embed.title(title);
        }
        if let Some(url) = &embed.url {
            new_embed = new_embed.url(url);
        }
        if let Some(description) = &embed.description {
            new_embed = new_embed.description(description);
        }
        // Intentionally skip image to remove it
        if let Some(footer) = &embed.footer {
            new_embed = new_embed.footer(serenity::all::CreateEmbedFooter::new(&footer.text));
        }
        if let Some(timestamp) = &embed.timestamp {
            new_embed = new_embed.timestamp(timestamp.clone());
        }

        // Copy fields
        for field in &embed.fields {
            new_embed = new_embed.field(&field.name, &field.value, field.inline);
        }

        // Update the message with the new black embed and remove all buttons
        let edit = EditMessage::new()
            .embed(new_embed)
            .components(vec![]); // Remove all components (buttons)

        message.clone().edit(&ctx.http, edit).await?;
    }

    // Tell discord we have handled the interaction
    component.create_response(&ctx.http, CreateInteractionResponse::Acknowledge).await?;

    Ok(())
}

async fn clear_bot_messages(ctx: &Context, channel_id: ChannelId) -> Result<usize, serenity::Error> {
    let mut count = 0;
    let current_user = ctx.http.get_current_user().await?;
    let bot_id = current_user.id;

    tracing::info!("Starting to clear bot messages from channel {} (bot ID: {})", channel_id, bot_id);

    // Fetch messages in batches
    let mut last_message_id = None;

    loop {
        let messages = if let Some(before_id) = last_message_id {
            channel_id.messages(&ctx.http, serenity::all::GetMessages::new().before(before_id).limit(100)).await?
        } else {
            channel_id.messages(&ctx.http, serenity::all::GetMessages::new().limit(100)).await?
        };

        if messages.is_empty() {
            break;
        }

        last_message_id = messages.last().map(|m| m.id);

        // Filter and delete bot messages
        for message in messages {
            if message.author.id == bot_id {
                match message.delete(&ctx.http).await {
                    Ok(_) => {
                        count += 1;
                        tracing::debug!("Deleted message {}", message.id);
                        // Add a small delay to avoid rate limiting
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to delete message {}: {:?}", message.id, e);
                    }
                }
            }
        }
    }

    tracing::info!("Cleared {} bot messages from channel {}", count, channel_id);
    Ok(count)
}

pub fn get_intents() -> GatewayIntents {
    GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGE_REACTIONS
}
