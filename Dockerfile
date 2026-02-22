# Docker image for colocfinder
#
# Build: docker build -t colocfinder .
#
# Run with environment variables (recommended for Docker):
#   docker run -d \
#     -e DISCORD_TOKEN=your_token_here \
#     -e CHANNEL_ID=123456789 \
#     -e INTERESTING_CHANNEL_ID=987654321 \
#     -e CHECK_INTERVAL_SECONDS=300 \
#     -e CITIES="Rennes,Paris,Lyon" \
#     -e TRACING_LEVEL=info \
#     -e REQUEST_DELAY_MS=2000 \
#     -e MAX_LISTING_AGE_MINUTES=1440 \
#     -e MIN_ROOMS=2 \
#     -v $(pwd)/data:/app/data \
#     colocfinder
#
# Or use docker-compose with .env file

FROM rust:1-alpine3.23 AS build
WORKDIR /app
COPY Cargo.* ./
RUN cargo fetch
COPY src src
RUN cargo build --release


FROM alpine:3.23 AS runtime
RUN addgroup -S appgroup && adduser -S -G appgroup appuser
WORKDIR /app
COPY --from=build /app/target/release/colocfinder ./
RUN chown -R appuser:appgroup /app
USER appuser
CMD ["./colocfinder"]