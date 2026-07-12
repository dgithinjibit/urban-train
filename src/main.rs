use anyhow::{anyhow, Context, Result};
use axum::{extract::State, response::Json, routing::get, serve, Router};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Serialize;
use std::{env, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::RwLock, time};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct AppState {
    client: Client,
    config: AppConfig,
    shared: Arc<RwLock<MonitorState>>,
}

#[derive(Clone, Debug)]
struct AppConfig {
    bind_addr: SocketAddr,
    pair_a: String,
    pair_b: String,
    history_window: usize,
    interval_minutes: u64,
    alert_threshold: f64,
    reset_threshold: f64,
    telegram_bot_token: Option<String>,
    telegram_chat_id: Option<String>,
}

#[derive(Default, Debug)]
struct MonitorState {
    last_zscore: Option<f64>,
    last_ratio: Option<f64>,
    last_run: Option<DateTime<Utc>>,
    last_error: Option<String>,
    alert_sent: bool,
}

#[derive(Serialize)]
struct StatusPayload {
    service: &'static str,
    pair_a: String,
    pair_b: String,
    history_window: usize,
    interval_minutes: u64,
    alert_threshold: f64,
    last_run: Option<DateTime<Utc>>,
    last_ratio: Option<f64>,
    last_zscore: Option<f64>,
    last_error: Option<String>,
    alert_sent: bool,
}

#[derive(Serialize)]
struct ZScorePayload {
    service: &'static str,
    pair_a: String,
    pair_b: String,
    last_run: Option<DateTime<Utc>>,
    last_ratio: Option<f64>,
    last_zscore: Option<f64>,
    last_error: Option<String>,
    alert_sent: bool,
}

async fn ping() -> &'static str {
    "OK"
}

async fn status(State(state): State<AppState>) -> Json<StatusPayload> {
    let snapshot = state.shared.read().await;
    Json(StatusPayload {
        service: "transmillion",
        pair_a: state.config.pair_a.clone(),
        pair_b: state.config.pair_b.clone(),
        history_window: state.config.history_window,
        interval_minutes: state.config.interval_minutes,
        alert_threshold: state.config.alert_threshold,
        last_run: snapshot.last_run,
        last_ratio: snapshot.last_ratio,
        last_zscore: snapshot.last_zscore,
        last_error: snapshot.last_error.clone(),
        alert_sent: snapshot.alert_sent,
    })
}

async fn zscore(State(state): State<AppState>) -> Json<ZScorePayload> {
    let snapshot = state.shared.read().await;
    Json(ZScorePayload {
        service: "transmillion",
        pair_a: state.config.pair_a.clone(),
        pair_b: state.config.pair_b.clone(),
        last_run: snapshot.last_run,
        last_ratio: snapshot.last_ratio,
        last_zscore: snapshot.last_zscore,
        last_error: snapshot.last_error.clone(),
        alert_sent: snapshot.alert_sent,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = load_config()?;
    let client = Client::builder().timeout(Duration::from_secs(15)).build()?;
    let shared = Arc::new(RwLock::new(MonitorState::default()));
    let app_state = AppState {
        client,
        config: config.clone(),
        shared: shared.clone(),
    };

    tokio::spawn(monitor_loop(app_state.clone()));

    let app = Router::new()
        .route("/ping", get(ping))
        .route("/status", get(status))
        .route("/zscore", get(zscore))
        .with_state(app_state);

    let listener = TcpListener::bind(config.bind_addr)
        .await
        .context("failed to bind socket")?;

    info!(%config.bind_addr, "starting HTTP server");
    serve(listener, app)
        .await
        .context("server stopped unexpectedly")?;

    Ok(())
}

fn load_config() -> Result<AppConfig> {
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = env::var("PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(10000);
    let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| format!("{}:{}", host, port));
    let bind_addr: SocketAddr = bind_addr.parse().context("invalid BIND_ADDR or PORT")?;

    let pair_a = env::var("PAIR_A_SYMBOL").unwrap_or_else(|_| "HYPEUSDT".to_string());
    let pair_b = env::var("PAIR_B_SYMBOL").unwrap_or_else(|_| "SOLUSDT".to_string());
    let history_window = env::var("HISTORY_WINDOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    let interval_minutes = env::var("INTERVAL_MINUTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let alert_threshold = env::var("ALERT_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2.0);
    let reset_threshold = env::var("RESET_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.5);

    let telegram_bot_token = env::var("TELEGRAM_BOT_TOKEN").ok();
    let telegram_chat_id = env::var("TELEGRAM_CHAT_ID").ok();

    Ok(AppConfig {
        bind_addr,
        pair_a,
        pair_b,
        history_window,
        interval_minutes,
        alert_threshold,
        reset_threshold,
        telegram_bot_token,
        telegram_chat_id,
    })
}

async fn monitor_loop(state: AppState) {
    let interval = Duration::from_secs(state.config.interval_minutes * 60);
    loop {
        let run_at = Utc::now();
        let result = run_cycle(&state).await;

        match result {
            Ok(_) => info!(timestamp = %run_at, "monitor cycle completed"),
            Err(err) => {
                let mut snapshot = state.shared.write().await;
                snapshot.last_run = Some(run_at);
                snapshot.last_error = Some(err.to_string());
                warn!(error = %err, "monitor cycle failed");
            }
        }

        time::sleep(interval).await;
    }
}

async fn run_cycle(state: &AppState) -> Result<()> {
    let pair_a = &state.config.pair_a;
    let pair_b = &state.config.pair_b;
    let window = state.config.history_window;

    let history_a = fetch_binance_history(&state.client, pair_a, window + 10).await?;
    let history_b = fetch_binance_history(&state.client, pair_b, window + 10).await?;

    let ratios = build_ratio_series(&history_a, &history_b)?;
    let zscore = compute_zscore(&ratios, window)?;

    let mut snapshot = state.shared.write().await;
    snapshot.last_run = Some(Utc::now());
    snapshot.last_ratio = Some(zscore.latest_ratio);
    snapshot.last_zscore = Some(zscore.zscore);
    snapshot.last_error = None;

    let should_alert = zscore.zscore.abs() >= state.config.alert_threshold && !snapshot.alert_sent;
    let should_reset = zscore.zscore.abs() < state.config.reset_threshold;

    if should_alert {
        let message = format!(
            "📈 Z-score alert for {} / {}:\nratio={:.6}\nzscore={:.2}\nwindow={}h\nthreshold={:.2}",
            pair_a,
            pair_b,
            zscore.latest_ratio,
            zscore.zscore,
            window,
            state.config.alert_threshold,
        );
        send_telegram(&state.config, &state.client, &message).await?;
        snapshot.alert_sent = true;
        info!(%pair_a, %pair_b, zscore = zscore.zscore, "alert sent");
    } else if should_reset {
        if snapshot.alert_sent {
            info!(zscore = zscore.zscore, "signal returned to neutral, alert reset");
        }
        snapshot.alert_sent = false;
    }

    Ok(())
}

async fn fetch_binance_history(client: &Client, symbol: &str, limit: usize) -> Result<Vec<f64>> {
    let url = format!(
        "https://api.binance.com/api/v3/klines?symbol={symbol}&interval=1h&limit={limit}",
        symbol = symbol,
        limit = limit.min(1000)
    );

    let response = client
        .get(url.clone())
        .send()
        .await
        .context("failed to fetch Binance klines")?;

    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read Binance response body")?;

    if !status.is_success() {
        return Err(anyhow!("Binance klines returned error status {}: {}", status, body));
    }

    let raw: Vec<Vec<serde_json::Value>> = serde_json::from_str(&body)
        .context("invalid Binance kline payload")?;
    let closes = raw
        .into_iter()
        .map(|row| {
            row.get(4)
                .and_then(|value| value.as_str())
                .and_then(|s| s.parse::<f64>().ok())
                .ok_or_else(|| anyhow!("invalid kline close price"))
        })
        .collect::<Result<Vec<f64>>>()?;

    if closes.len() < 2 {
        return Err(anyhow!("not enough Binance history points"));
    }

    Ok(closes)
}

fn build_ratio_series(history_a: &[f64], history_b: &[f64]) -> Result<Vec<f64>> {
    let len = std::cmp::min(history_a.len(), history_b.len());
    if len < 2 {
        return Err(anyhow!("not enough matched history for ratio series"));
    }

    Ok((0..len)
        .map(|index| history_a[index] / history_b[index])
        .collect())
}

struct ZScore {
    latest_ratio: f64,
    zscore: f64,
}

fn compute_zscore(ratios: &[f64], window: usize) -> Result<ZScore> {
    if ratios.len() < 2 {
        return Err(anyhow!("not enough ratio values to compute z-score"));
    }

    let available = ratios.len() - 1;
    let sample_len = std::cmp::min(window, available);
    if sample_len < 2 {
        return Err(anyhow!("not enough points in z-score sample"));
    }

    let sample_start = available - sample_len;
    let sample = &ratios[sample_start..available];
    let latest_ratio = ratios[available];
    let mean = sample.iter().copied().sum::<f64>() / sample_len as f64;
    let variance = sample
        .iter()
        .map(|value| {
            let delta = value - mean;
            delta * delta
        })
        .sum::<f64>()
        / sample_len as f64;
    let stddev = variance.sqrt();

    if stddev == 0.0 {
        return Err(anyhow!("z-score sample variance is zero"));
    }

    Ok(ZScore {
        latest_ratio,
        zscore: (latest_ratio - mean) / stddev,
    })
}

async fn send_telegram(config: &AppConfig, client: &Client, text: &str) -> Result<()> {
    let bot_token = match &config.telegram_bot_token {
        Some(token) => token,
        None => {
            warn!("TELEGRAM_BOT_TOKEN is not configured; skipping Telegram alert");
            return Ok(());
        }
    };

    let chat_id = match &config.telegram_chat_id {
        Some(chat_id) => chat_id,
        None => {
            warn!("TELEGRAM_CHAT_ID is not configured; skipping Telegram alert");
            return Ok(());
        }
    };

    let url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        bot_token
    );

    let payload = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
    });

    let response = client.post(url).json(&payload).send().await?;
    response.error_for_status().context("Telegram rejected the alert")?;
    Ok(())
}
