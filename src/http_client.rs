use reqwest::{Client, header, cookie::Jar};
use anyhow::Result;
use std::sync::Arc;

/// Creates an HTTP client configured to avoid CAPTCHA and bot detection
/// Returns both the client and the cookie jar for persistence
pub fn create_http_client(user_agent: &str) -> Result<Client> {
    create_http_client_with_cookies(user_agent, None)
}

/// Creates an HTTP client with optional cookie jar for cookie persistence
pub fn create_http_client_with_cookies(user_agent: &str, cookie_jar: Option<Arc<Jar>>) -> Result<Client> {
    let mut headers = header::HeaderMap::new();

    // Standard browser headers to look more like a real browser
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8")
    );
    headers.insert(
        header::ACCEPT_LANGUAGE,
        header::HeaderValue::from_static("en-US,en;q=0.9,fr;q=0.8")
    );
    headers.insert(
        header::ACCEPT_ENCODING,
        header::HeaderValue::from_static("gzip, deflate, br")
    );
    headers.insert(
        header::DNT,
        header::HeaderValue::from_static("1")
    );
    headers.insert(
        header::CONNECTION,
        header::HeaderValue::from_static("keep-alive")
    );
    headers.insert(
        "Upgrade-Insecure-Requests",
        header::HeaderValue::from_static("1")
    );
    headers.insert(
        "Sec-Fetch-Dest",
        header::HeaderValue::from_static("document")
    );
    headers.insert(
        "Sec-Fetch-Mode",
        header::HeaderValue::from_static("navigate")
    );
    headers.insert(
        "Sec-Fetch-Site",
        header::HeaderValue::from_static("none")
    );
    headers.insert(
        "Sec-Fetch-User",
        header::HeaderValue::from_static("?1")
    );
    headers.insert(
        "Cache-Control",
        header::HeaderValue::from_static("max-age=0")
    );

    let mut builder = Client::builder()
        .user_agent(user_agent)
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(30));

    // Add cookie jar if provided, otherwise create a new one
    if let Some(jar) = cookie_jar {
        builder = builder.cookie_provider(jar);
    } else {
        builder = builder.cookie_store(true);
    }

    let client = builder.build()?;

    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper function to detect if HTML content is a CAPTCHA page
    fn is_captcha_page(html: &str) -> bool {
        let html_lower = html.to_lowercase();

        // Check for common CAPTCHA indicators
        html_lower.contains("captcha") ||
        html_lower.contains("cloudflare") ||
        html_lower.contains("challenge") ||
        html_lower.contains("bot detection") ||
        html_lower.contains("access denied") ||
        html_lower.contains("blocked") ||
        // Check for CAPTCHA-related scripts
        html_lower.contains("recaptcha") ||
        html_lower.contains("hcaptcha") ||
        // Check for Cloudflare challenge
        html_lower.contains("cf-browser-verification") ||
        html_lower.contains("cf_chl_opt")
    }

    /// Helper function to check if HTML looks like a real Leboncoin page
    fn is_valid_leboncoin_page(html: &str) -> bool {
        // Check for Leboncoin-specific elements that indicate a real page
        html.contains("leboncoin") &&
        (html.contains("data-qa-id") || html.contains("article") || html.contains("search"))
    }

    #[tokio::test]
    async fn test_leboncoin_returns_actual_content_not_captcha() {
        let user_agent = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
        let client = create_http_client(user_agent)
            .expect("Failed to create HTTP client");

        let url = "https://www.leboncoin.fr/recherche?category=10&locations=Paris";

        let response = client.get(url)
            .send()
            .await
            .expect("Failed to send request to Leboncoin");

        assert!(response.status().is_success(),
            "Leboncoin request should return success status, got: {}", response.status());

        let html = response.text().await.expect("Failed to get response text");

        assert!(!html.is_empty(), "Response HTML should not be empty");
        assert!(!is_captcha_page(&html),
            "Leboncoin returned a CAPTCHA page instead of actual content. Our HTTP client may be detected as a bot.");
        assert!(is_valid_leboncoin_page(&html),
            "Response doesn't look like a valid Leboncoin page");
    }

    #[tokio::test]
    async fn test_leboncoin_search_contains_listings() {
        let user_agent = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
        let client = create_http_client(user_agent)
            .expect("Failed to create HTTP client");

        let url = "https://www.leboncoin.fr/recherche?category=10&locations=Lyon";

        let response = client.get(url)
            .send()
            .await
            .expect("Failed to send request");

        let html = response.text().await.expect("Failed to get response text");

        // Check that we're not blocked by CAPTCHA
        assert!(!is_captcha_page(&html), "Got CAPTCHA page instead of search results");

        // Check for common listing indicators
        let has_listings = html.contains("data-qa-id=\"aditem") ||
                          html.contains("<article") ||
                          html.contains("adCard");

        assert!(has_listings,
            "Leboncoin search page should contain listing elements. Page may have changed structure.");
    }

    #[tokio::test]
    async fn test_http_client_handles_redirects() {
        let user_agent = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
        let client = create_http_client(user_agent)
            .expect("Failed to create HTTP client");

        // HTTP should redirect to HTTPS
        let response = client.get("http://www.leboncoin.fr")
            .send()
            .await
            .expect("Failed to handle redirect");

        assert!(response.status().is_success(),
            "Client should successfully handle redirects");
    }

    #[tokio::test]
    async fn test_http_client_timeout_works() {
        let user_agent = "Mozilla/5.0 (Test Agent)";
        let client = create_http_client(user_agent)
            .expect("Failed to create HTTP client");

        // Try to connect to a non-routable IP (should timeout)
        let result = client.get("http://10.255.255.1")
            .send()
            .await;

        // Should fail with timeout or connection error, not hang indefinitely
        assert!(result.is_err(), "Request to non-routable IP should fail/timeout");
    }

    #[tokio::test]
    async fn test_http_client_with_different_user_agents() {
        // Test that different user agents can create clients successfully
        let user_agents = vec![
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
        ];

        for ua in user_agents {
            let client = create_http_client(ua);
            assert!(client.is_ok(), "Failed to create client with user agent: {}", ua);
        }
    }

    #[test]
    fn test_create_http_client_succeeds() {
        let user_agent = "Mozilla/5.0 (Test Agent)";
        let result = create_http_client(user_agent);

        assert!(result.is_ok(), "Client creation should succeed");
    }
}
