# Colocfinder

A Discord bot that scrapes rental listings from Leboncoin and posts them to Discord channels.

## Features

- Scrapes Leboncoin for new rental listings
- Posts listings to Discord with embeds
- Interactive buttons to mark listings as "Interesting" or "Not Good"
- Automatic filtering by minimum number of rooms
- Configurable maximum listing age
- Cookie support for bypassing captchas

## Directory Structure

```
colocfinder/
├── data/                      # All configuration and runtime data
│   ├── config.yaml           # Your configuration (create from config.example.yaml)
│   ├── config.example.yaml   # Example configuration file
│   ├── cookies.json          # Browser cookies for bypassing captchas (optional)
│   └── listings.db           # SQLite database of scraped listings
├── src/                       # Source code
└── ...
```

The `data/` directory contains all configuration files and runtime data. This keeps the project root clean and makes it easy to backup or mount as a Docker volume.

## Configuration

Colocfinder can be configured using either a YAML configuration file or environment variables. Environment variables take precedence over the config file values, making it easy to use with Docker.

All configuration files and runtime data are stored in the `data/` directory.

### Configuration File (data/config.yaml)

Copy `data/config.example.yaml` to `data/config.yaml` and edit the values:

```yaml
discord_token: YOUR_DISCORD_BOT_TOKEN
channel_id: 123456789
interesting_channel_id: 987654321
check_interval_seconds: 300
tracing_level: info
user_agent: 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36'
request_delay_ms: 2000
max_listing_age_minutes: 1440
min_rooms: 2
cities:
  - Rennes
  - Paris
```

### Environment Variables

All configuration options can be set via environment variables:

| Environment Variable | Description | Required | Default |
|---------------------|-------------|----------|---------|
| `DISCORD_TOKEN` | Discord bot token | Yes | - |
| `CHANNEL_ID` | Main channel ID for all posts | Yes | - |
| `INTERESTING_CHANNEL_ID` | Channel ID for interesting posts | Yes | - |
| `CHECK_INTERVAL_SECONDS` | How often to check for new listings | No | 300 |
| `CITIES` | Comma-separated list of cities (e.g., "Rennes,Paris,Lyon") | Yes | - |
| `TRACING_LEVEL` | Logging level (trace, debug, info, warn, error) | No | info |
| `USER_AGENT` | HTTP User-Agent string | No | Mozilla/5.0... |
| `REQUEST_DELAY_MS` | Delay between requests in milliseconds | No | 2000 |
| `MAX_LISTING_AGE_MINUTES` | Only show listings from last X minutes | No | 1440 |
| `MIN_ROOMS` | Minimum number of rooms | No | 1 |

## Running

### Local Development

```bash
# Build the project
cargo build --release

# Run with config file
./target/release/colocfinder

# Run with environment variables
DISCORD_TOKEN=your_token CHANNEL_ID=123 INTERESTING_CHANNEL_ID=456 CITIES=Rennes ./target/release/colocfinder
```

### Docker

#### Using docker run

```bash
docker build -t colocfinder .

docker run -d \
  -e DISCORD_TOKEN=your_token_here \
  -e CHANNEL_ID=123456789 \
  -e INTERESTING_CHANNEL_ID=987654321 \
  -e CHECK_INTERVAL_SECONDS=300 \
  -e CITIES="Rennes,Paris,Lyon" \
  -e TRACING_LEVEL=info \
  -e REQUEST_DELAY_MS=2000 \
  -e MAX_LISTING_AGE_MINUTES=1440 \
  -e MIN_ROOMS=2 \
  -v $(pwd)/data:/app/data \
  colocfinder
```

#### Using docker-compose

1. Copy `.env.example` to `.env`:
   ```bash
   cp .env.example .env
   ```

2. Edit `.env` and fill in your values:
   ```bash
   DISCORD_TOKEN=YOUR_DISCORD_BOT_TOKEN
   CHANNEL_ID=123456789
   INTERESTING_CHANNEL_ID=987654321
   # ... other values
   ```

3. Run with docker-compose:
   ```bash
   docker-compose up -d
   ```

4. View logs:
   ```bash
   docker-compose logs -f
   ```

## Cookie Handling

To bypass Leboncoin captchas, you can provide cookies in a `data/cookies.json` file. See `COOKIES_GUIDE.md` for instructions on how to obtain cookies.

## Commands

The bot supports the following Discord commands:

- `/clear` - Removes all messages from the bot in the current channel

## Supported Cities

The bot supports the following cities (case-sensitive):
- Rennes
- Paris
- Lyon
- Marseille
- Toulouse
- Nice
- Nantes
- Bordeaux
- Lille
- Strasbourg

## License

MIT

