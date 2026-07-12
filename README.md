# Transmillion

A Rust-based trading signal monitor designed for Render with a keep-alive endpoint and Telegram alerts.

## What this repo contains

- `src/main.rs` — an Axum web service with `/ping` and `/status` endpoints
- `Cargo.toml` — Rust dependencies for HTTP, async, logging, and Binance API access
- `.github/workflows/keep-alive.yml` — optional GitHub Actions workflow to ping your Render app
- `README.md` — deployment, environment, and keep-alive guidance

## Key capabilities

- `GET /ping` keeps your Render free-tier instance awake when polled by UptimeRobot, cron-job.org, or GitHub Actions
- background monitor task fetches Binance 1h candle data for two symbols
- computes a Z-score on the ratio series and detects threshold crossovers
- optional Telegram alerting when a signal triggers
- `GET /status` and `GET /zscore` return the latest monitoring state

## Environment variables

Set these values in Render, GitHub Actions secrets, or a local `.env` file:

```text
HOST=0.0.0.0
PORT=10000
PAIR_A_SYMBOL=HYPEUSDT
PAIR_B_SYMBOL=SOLUSDT
HISTORY_WINDOW=24
INTERVAL_MINUTES=30
ALERT_THRESHOLD=2.0
RESET_THRESHOLD=1.5
TELEGRAM_BOT_TOKEN=your_bot_token
TELEGRAM_CHAT_ID=your_chat_id
```

### Notes

- `PAIR_A_SYMBOL` and `PAIR_B_SYMBOL` are Binance symbol names. Default is `HYPEUSDT` and `SOLUSDT`.
- `HISTORY_WINDOW` controls how many hourly ratio points are used to compute the mean/stddev.
- `INTERVAL_MINUTES` sets the monitor cadence.
- Telegram only runs if both `TELEGRAM_BOT_TOKEN` and `TELEGRAM_CHAT_ID` are configured.

## Deploying to Render

1. Create a **Web Service** on Render.
2. Connect your GitHub repo.
3. Set the build command to:

```bash
cargo build --release
```

4. Set the start command to:

```bash
./target/release/transmillion
```

5. Add the environment variables above.
6. Confirm the service is reachable at `https://<your-app>.onrender.com/ping`.

## Keep the Render app awake

Render free tier sleeps after inactivity. Use one of these free keep-alive options:

1. **UptimeRobot**
   - Create an HTTP monitor for `https://<your-app>.onrender.com/ping`
   - Use a 5-minute interval

2. **cron-job.org**
   - Create a cron job pointing to `/ping`
   - Use a 10-minute interval

3. **GitHub Actions**
   - Replace `https://<your-app>.onrender.com/ping` in `.github/workflows/keep-alive.yml`
   - This is useful, but GitHub Actions can pause if the repo is inactive for 60 days.

> Best practice: use UptimeRobot as the primary keep-alive method, and GitHub Actions or cron-job.org as backup.

## GitHub Actions keep-alive

The workflow in `.github/workflows/keep-alive.yml` will ping your app every 10 minutes. Update the URL to your production Render URL before you enable it.

## Binance and Coingecko

This project currently uses Binance public price endpoints for signal generation. Binance public ticker and kline calls do not require an API key for price monitoring.

If you want a second price source or redundancy, Coingecko can be added later as a fallback.

## Next steps

- Push this repo to GitHub
- Deploy on Render and configure the environment variables
- Set up UptimeRobot or cron-job.org to hit `/ping`
- Verify `GET /status` returns the latest monitoring state

## Improvements you can add later

- Binance order execution with API key / secret
- coinglass / tradingview signal inputs
- multi-symbol portfolio monitoring
- Webhook dashboard for Telegram, Discord, or Slack alerts
- more advanced statistical models beyond simple ratio Z-score
