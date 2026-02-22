# Cookie Export Guide

If you're encountering CAPTCHAs when scraping Leboncoin, you can export cookies from your browser to try to bypass them.

The `data/cookies.json` file should be an array of cookie objects:

```json
[
  {
    "name": "cookiename",
    "value": "cookievalue",
    "domain": ".leboncoin.fr",
    "path": "/",
    "expires": 1234567890,
    "httpOnly": false,
    "secure": true
  }
]
```

When you run the bot, you should see this in the logs:

```
INFO colocfinder: Successfully loaded cookies from data/cookies.json
INFO colocfinder::scrapers::leboncoin: Loaded 15 cookies from data/cookies.json
```

Cookies will eventually expire.
