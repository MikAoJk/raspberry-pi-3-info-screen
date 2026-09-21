mod log;

use std::{env, fs, sync::Arc, time::{SystemTime, UNIX_EPOCH}};

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use ::log::info;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::log::init_log4rs;

const WEATHER_CACHE_TTL: u64 = 15 * 60;
const CALENDAR_CACHE_TTL: u64 = 15 * 60;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_log4rs();

    let application_state = ApplicationState::new();

    let app = Router::new()
        .route("/", get(root))
        .route("/oauth/callback", get(google_oauth_callback))
        .route("/api/weather", get(get_weather))
        .route("/api/calendar", get(get_calendar))
        .route("/api/dashboard", get(get_dashboard))
        .route("/api/oauth/google/config", get(get_google_oauth_config))
        .route("/api/oauth/google/token", post(store_google_token))
        .route("/api/oauth/google/refresh", post(refresh_google_token))
        .with_state(application_state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;

    info!("listening on {}", listener.local_addr()?);

    axum::serve(listener, app).await?;

    Ok(())
}

#[derive(Clone)]
pub struct ApplicationState {
    config: Arc<AppConfig>,
    http_client: reqwest::Client,
    weather_cache: Arc<Mutex<Option<WeatherCache>>>,
    calendar_cache: Arc<Mutex<Option<CalendarCache>>>,
    oauth_tokens: Arc<Mutex<Option<OAuthToken>>>,
}

impl ApplicationState {
    fn new() -> Self {
        Self {
            config: Arc::new(AppConfig::from_env()),
            http_client: reqwest::Client::new(),
            weather_cache: Arc::new(Mutex::new(None)),
            calendar_cache: Arc::new(Mutex::new(None)),
            oauth_tokens: Arc::new(Mutex::new(None)),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AppConfig {
    weather_latitude: String,
    weather_longitude: String,
    calendar_id: String,
    google_client_id: String,
    google_client_secret: String,
    google_redirect_uri: String,
}

impl AppConfig {
    fn from_env() -> Self {
        Self {
            weather_latitude: env::var("WEATHER_LATITUDE").unwrap_or_else(|_| "58.14574000943632".to_string()),
            weather_longitude: env::var("WEATHER_LONGITUDE").unwrap_or_else(|_| "8.06137337891726".to_string()),
            calendar_id: env::var("CALENDAR_ID").unwrap_or_else(|_| "f2469c230e7c5d747c481a395b71f16b70f6b4c8d20d2bcb4348bc4e34eb814f@group.calendar.google.com".to_string()),
            google_client_id: env::var("GOOGLE_CLIENT_ID").unwrap_or_default(),
            google_client_secret: env::var("GOOGLE_CLIENT_SECRET").unwrap_or_default(),
            google_redirect_uri: env::var("GOOGLE_REDIRECT_URI").unwrap_or_else(|_| "http://localhost:8080/oauth/callback".to_string()),
        }
    }

    fn has_google_oauth_config(&self) -> bool {
        !self.google_client_id.trim().is_empty() && !self.google_client_secret.trim().is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WeatherCache {
    cached_at_unix: u64,
    payload: WeatherSnapshot,
}

impl WeatherCache {
    fn is_fresh(&self, ttl_seconds: u64) -> bool {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        now.saturating_sub(self.cached_at_unix) < ttl_seconds
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CalendarCache {
    cached_at_unix: u64,
    payload: CalendarAggregation,
}

impl CalendarCache {
    fn is_fresh(&self, ttl_seconds: u64) -> bool {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        now.saturating_sub(self.cached_at_unix) < ttl_seconds
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WeatherSnapshot {
    summary: String,
    icon: String,
    current_temperature_c: f64,
    hourly: Vec<HourlyWeather>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct HourlyWeather {
    time: String,
    temperature_c: f64,
    icon: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CalendarAggregation {
    auth_required: bool,
    message: Option<String>,
    days: Vec<CalendarDay>,
    fetched_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CalendarDay {
    date: String,
    events: Vec<CalendarEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CalendarEvent {
    summary: String,
    start: String,
    end: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DashboardResponse {
    weather: WeatherSnapshot,
    calendar: CalendarAggregation,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OAuthToken {
    access_token: String,
    refresh_token: Option<String>,
    token_type: String,
    expires_at_unix: u64,
}

impl OAuthToken {
    fn is_expired(&self) -> bool {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        now >= self.expires_at_unix
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OAuthTokenRequest {
    code: String,
    redirect_uri: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GoogleOAuthConfigResponse {
    client_id: String,
    redirect_uri: String,
    scope: String,
}

#[derive(Clone, Debug, Deserialize)]
struct GoogleOAuthCallbackQuery {
    code: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OAuthRefreshRequest {
    refresh_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OAuthExchangeResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    token_type: String,
}

async fn root() -> Html<&'static str> {
    let html_in_string: String = fs::read_to_string("static/index.html").expect("failed to read html file to string");
    let html_in_str: &str = string_to_static_str(html_in_string);

    Html(html_in_str)
}

async fn get_google_oauth_config(State(state): State<ApplicationState>) -> Result<Json<GoogleOAuthConfigResponse>, (StatusCode, String)> {
    let client_id = state.config.google_client_id.clone();
    let redirect_uri = state.config.google_redirect_uri.clone();

    if !state.config.has_google_oauth_config() {
        return Err((StatusCode::BAD_REQUEST, "Google OAuth is not configured on the server.".to_string()));
    }

    Ok(Json(GoogleOAuthConfigResponse {
        client_id,
        redirect_uri,
        scope: "https://www.googleapis.com/auth/calendar.readonly".to_string(),
    }))
}

async fn google_oauth_callback(
    State(state): State<ApplicationState>,
    Query(query): Query<GoogleOAuthCallbackQuery>,
) -> Result<Html<String>, (StatusCode, String)> {
    if let Some(error) = query.error {
        return Err((StatusCode::BAD_REQUEST, format!("Google OAuth denied: {error}")));
    }

    let code = query.code.ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing Google OAuth code.".to_string()))?;
    let redirect_uri = state.config.google_redirect_uri.clone();
    let token = exchange_google_code_for_token(&state, code, redirect_uri).await?;
    *state.oauth_tokens.lock().await = Some(token);

    let html = r#"
        <!doctype html>
        <html lang="en">
        <head>
            <meta charset="utf-8" />
            <meta http-equiv="refresh" content="0; url=/" />
            <title>Google Calendar authorization</title>
        </head>
        <body>
            <p>Google Calendar authorization complete. Redirecting...</p>
        </body>
        </html>
    "#;

    Ok(Html(html.to_string()))
}

fn string_to_static_str(s: String) -> &'static str {
    s.leak()
}


async fn get_weather(State(state): State<ApplicationState>) -> Result<Json<WeatherSnapshot>, (StatusCode, String)> {
    let cached_weather = {
        let cache = state.weather_cache.lock().await;
        cache.clone().filter(|entry| entry.is_fresh(WEATHER_CACHE_TTL))
    };

    if let Some(cached) = cached_weather {
        return Ok(Json(cached.payload));
    }

    let payload = fetch_weather_from_met(&state).await.map_err(|error| {
        (StatusCode::BAD_GATEWAY, format!("Weather request failed: {error}"))
    })?;

    let cache_entry = WeatherCache {
        cached_at_unix: now_unix_seconds(),
        payload: payload.clone(),
    };

    *state.weather_cache.lock().await = Some(cache_entry);
    Ok(Json(payload))
}

async fn get_calendar(State(state): State<ApplicationState>) -> Result<Json<CalendarAggregation>, (StatusCode, String)> {
    let cached_calendar = {
        let cache = state.calendar_cache.lock().await;
        cache.clone().filter(|entry| entry.is_fresh(CALENDAR_CACHE_TTL))
    };

    if let Some(cached) = cached_calendar {
        return Ok(Json(cached.payload));
    }

    let payload = fetch_calendar_aggregation(&state).await.map_err(|error| {
        (StatusCode::BAD_GATEWAY, format!("Calendar request failed: {error}"))
    })?;

    let cache_entry = CalendarCache {
        cached_at_unix: now_unix_seconds(),
        payload: payload.clone(),
    };

    *state.calendar_cache.lock().await = Some(cache_entry);
    Ok(Json(payload))
}

async fn get_dashboard(State(state): State<ApplicationState>) -> Result<Json<DashboardResponse>, (StatusCode, String)> {
    let weather = fetch_weather_snapshot(&state).await.map_err(|error| (StatusCode::BAD_GATEWAY, error))?;
    let calendar = fetch_calendar_aggregation(&state).await.map_err(|error| (StatusCode::BAD_GATEWAY, error))?;

    Ok(Json(DashboardResponse { weather, calendar }))
}

async fn fetch_weather_snapshot(state: &ApplicationState) -> Result<WeatherSnapshot, String> {
    if let Some(cached) = state.weather_cache.lock().await.clone().filter(|entry| entry.is_fresh(WEATHER_CACHE_TTL)) {
        return Ok(cached.payload);
    }

    let fresh = fetch_weather_from_met(state).await?;
    *state.weather_cache.lock().await = Some(WeatherCache { cached_at_unix: now_unix_seconds(), payload: fresh.clone() });
    Ok(fresh)
}

async fn fetch_calendar_aggregation(state: &ApplicationState) -> Result<CalendarAggregation, String> {
    if let Some(cached) = state.calendar_cache.lock().await.clone().filter(|entry| entry.is_fresh(CALENDAR_CACHE_TTL)) {
        return Ok(cached.payload);
    }

    let token = {
        let guard = state.oauth_tokens.lock().await;
        guard.clone()
    };

    let tokens = match token {
        Some(token) => token,
        None => {
            return Ok(CalendarAggregation {
                auth_required: true,
                message: Some("Google Calendar authorization required.".to_string()),
                days: vec![],
                fetched_at: DateTime::<Utc>::from_timestamp(now_unix_seconds() as i64, 0).unwrap_or_else(Utc::now).to_rfc3339(),
            });
        }
    };

    let access_token = if tokens.is_expired() {
        let refreshed = refresh_google_access_token(state, tokens.refresh_token.clone().unwrap_or_default()).await?;
        let mut lock = state.oauth_tokens.lock().await;
        *lock = Some(refreshed.clone());
        refreshed.access_token
    } else {
        tokens.access_token
    };

    let calendar_id = state.config.calendar_id.clone();
    let start = chrono::Utc::now().with_timezone(&chrono::Utc).date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let end = start + chrono::Duration::days(7);
    let response = state.http_client
        .get(&format!("https://www.googleapis.com/calendar/v3/calendars/{calendar_id}/events"))
        .query(&[
            ("timeMin", start.to_rfc3339().to_string()),
            ("timeMax", end.to_rfc3339().to_string()),
            ("singleEvents", "true".to_string()),
            ("orderBy", "startTime".to_string()),
            ("maxResults", "100".to_string()),
            ("showDeleted", "false".to_string()),
        ])
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| format!("calendar API request failed: {error}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("calendar API returned {status}: {body}"));
    }

    let payload: GoogleCalendarApiResponse = response.json().await.map_err(|error| format!("calendar API parse failed: {error}"))?;
    let days = build_calendar_days(payload.items);
    let output = CalendarAggregation {
        auth_required: false,
        message: None,
        days,
        fetched_at: DateTime::<Utc>::from_timestamp(now_unix_seconds() as i64, 0).unwrap_or_else(Utc::now).to_rfc3339(),
    };

    *state.calendar_cache.lock().await = Some(CalendarCache { cached_at_unix: now_unix_seconds(), payload: output.clone() });
    Ok(output)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GoogleCalendarApiResponse {
    #[serde(default)] items: Vec<GoogleCalendarItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GoogleCalendarItem {
    #[serde(default)] summary: Option<String>,
    #[serde(default)] start: GoogleEventTime,
    #[serde(default)] end: Option<GoogleEventTime>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct GoogleEventTime {
    #[serde(default)] date: Option<String>,
    #[serde(default, rename = "dateTime")] date_time: Option<String>,
}

fn build_calendar_days(items: Vec<GoogleCalendarItem>) -> Vec<CalendarDay> {
    let mut events_by_day: std::collections::BTreeMap<String, Vec<CalendarEvent>> = std::collections::BTreeMap::new();

    for item in items {
        let start_value = item.start.date.clone().or_else(|| item.start.date_time.clone());
        let Some(date_value) = start_value else { continue; };
        let date_key = if date_value.len() >= 10 {
            date_value[..10].to_string()
        } else {
            date_value
        };

        let start_text = if item.start.date_time.is_some() {
            item.start.date_time.clone().unwrap_or_default()
        } else {
            item.start.date.clone().unwrap_or_default()
        };

        let end_text = item.end.and_then(|value| value.date_time.or(value.date));
        let event = CalendarEvent {
            summary: item.summary.unwrap_or_else(|| "Untitled event".to_string()),
            start: start_text,
            end: end_text,
        };

        events_by_day.entry(date_key).or_default().push(event);
    }

    let start = chrono::Utc::now().date_naive();
    let mut days = Vec::new();
    for offset in 0..7 {
        let date = start + chrono::Duration::days(offset);
        let key = date.to_string();
        let events = events_by_day.remove(&key).unwrap_or_default();
        days.push(CalendarDay {
            date: key,
            events,
        });
    }
    days
}

#[derive(Clone, Debug, Deserialize)]
struct MetNoLocationForecastResponse {
    properties: MetNoProperties,
}

#[derive(Clone, Debug, Deserialize)]
struct MetNoProperties {
    timeseries: Vec<MetNoTimeSeries>,
}

#[derive(Clone, Debug, Deserialize)]
struct MetNoTimeSeries {
    time: String,
    data: MetNoData,
}

#[derive(Clone, Debug, Deserialize)]
struct MetNoData {
    instant: MetNoInstant,
    next_1_hours: Option<MetNoNextHourSummary>,
}

#[derive(Clone, Debug, Deserialize)]
struct MetNoInstant {
    details: MetNoDetails,
}

#[derive(Clone, Debug, Deserialize)]
struct MetNoDetails {
    air_temperature: f64,
}

#[derive(Clone, Debug, Deserialize)]
struct MetNoNextHourSummary {
    summary: MetNoSummary,
}

#[derive(Clone, Debug, Deserialize)]
struct MetNoSummary {
    symbol_code: Option<String>,
}

async fn fetch_weather_from_met(state: &ApplicationState) -> Result<WeatherSnapshot, String> {
    let url = format!(
        "https://api.met.no/weatherapi/locationforecast/2.0/compact?lat={}&lon={}",
        state.config.weather_latitude,
        state.config.weather_longitude
    );

    let response = state.http_client
        .get(url)
        .header("User-Agent", "raspberry-pi-info-screen/1.0")
        .send()
        .await
        .map_err(|error| format!("weather request failed: {error}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("weather API returned {status}: {body}"));
    }

    let payload: MetNoLocationForecastResponse = response.json().await.map_err(|error| format!("weather JSON decode failed: {error}"))?;
    let timeseries = payload.properties.timeseries;

    let current = timeseries.first().cloned().ok_or_else(|| "weather data was empty".to_string())?;
    let temperature = current.data.instant.details.air_temperature;
    let symbol = current.data.next_1_hours.as_ref().and_then(|summary| summary.summary.symbol_code.as_deref()).unwrap_or("clearsky_day");

    let hourly = timeseries
        .into_iter()
        .take(24)
        .map(|entry| HourlyWeather {
            time: entry.time,
            temperature_c: entry.data.instant.details.air_temperature,
            icon: resolve_weather_icon(entry.data.next_1_hours.as_ref().and_then(|summary| summary.summary.symbol_code.as_deref()).unwrap_or("clearsky_day")),
        })
        .collect();

    Ok(WeatherSnapshot {
        summary: format!("Nå: {temperature}°C"),
        icon: resolve_weather_icon(symbol),
        current_temperature_c: temperature,
        hourly,
    })
}

fn resolve_weather_icon(symbol_code: &str) -> String {
    let icons = std::collections::HashMap::from([
        ("clearsky_day", "☀️"),
        ("clearsky_night", "🌙"),
        ("fair_day", "🌤️"),
        ("fair_night", "🌤️"),
        ("partlycloudy_day", "⛅"),
        ("partlycloudy_night", "⛅"),
        ("cloudy", "☁️"),
        ("fog", "🌫️"),
        ("lightrain", "🌦️"),
        ("rain", "🌧️"),
        ("heavyrain", "🌧️"),
        ("rainshowers_day", "🌦️"),
        ("rainshowers_night", "🌦️"),
        ("heavyrainshowers_day", "⛈️"),
        ("heavyrainshowers_night", "⛈️"),
        ("lightsnow", "🌨️"),
        ("snow", "❄️"),
        ("heavysnow", "❄️"),
        ("sleet", "🌧️"),
        ("thunderstorms", "⛈️"),
    ]);

    icons.get(symbol_code).copied().unwrap_or("🌤️").to_string()
}

async fn store_google_token(
    State(state): State<ApplicationState>,
    Json(request): Json<OAuthTokenRequest>,
) -> Result<Json<OAuthToken>, (StatusCode, String)> {
    let redirect_uri = request.redirect_uri.clone().unwrap_or_else(|| state.config.google_redirect_uri.clone());
    let token = exchange_google_code_for_token(&state, request.code, redirect_uri).await?;
    *state.oauth_tokens.lock().await = Some(token.clone());
    Ok(Json(token))
}

async fn exchange_google_code_for_token(
    state: &ApplicationState,
    code: String,
    redirect_uri: String,
) -> Result<OAuthToken, (StatusCode, String)> {
    let client_id = state.config.google_client_id.clone();
    let client_secret = state.config.google_client_secret.clone();

    if !state.config.has_google_oauth_config() {
        return Err((StatusCode::BAD_REQUEST, "Google OAuth is not configured on the server.".to_string()));
    }

    let payload = vec![
        ("code", code.as_str()),
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("grant_type", "authorization_code"),
    ];

    let response = state.http_client
        .post("https://oauth2.googleapis.com/token")
        .form(&payload)
        .send()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("token exchange failed: {error}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err((StatusCode::BAD_GATEWAY, format!("token exchange returned {status}: {body}")));
    }

    let oauth_response: OAuthExchangeResponse = response.json().await.map_err(|error| {
        (StatusCode::BAD_GATEWAY, format!("token response could not be parsed: {error}"))
    })?;

    let expires_in = oauth_response.expires_in.unwrap_or(3600);
    Ok(OAuthToken {
        access_token: oauth_response.access_token,
        refresh_token: oauth_response.refresh_token,
        token_type: oauth_response.token_type,
        expires_at_unix: now_unix_seconds().saturating_add(expires_in),
    })
}

async fn refresh_google_token(
    State(state): State<ApplicationState>,
    Json(request): Json<OAuthRefreshRequest>,
) -> Result<Json<OAuthToken>, (StatusCode, String)> {
    let client_id = state.config.google_client_id.clone();
    let client_secret = state.config.google_client_secret.clone();

    if !state.config.has_google_oauth_config() {
        return Err((StatusCode::BAD_REQUEST, "Google OAuth is not configured on the server.".to_string()));
    }

    let payload = vec![
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
        ("refresh_token", request.refresh_token.as_str()),
        ("grant_type", "refresh_token"),
    ];

    let response = state.http_client
        .post("https://oauth2.googleapis.com/token")
        .form(&payload)
        .send()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("token refresh failed: {error}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err((StatusCode::BAD_GATEWAY, format!("token refresh returned {status}: {body}")));
    }

    let oauth_response: OAuthExchangeResponse = response.json().await.map_err(|error| {
        (StatusCode::BAD_GATEWAY, format!("refresh response could not be parsed: {error}"))
    })?;

    let token = OAuthToken {
        access_token: oauth_response.access_token,
        refresh_token: Some(request.refresh_token),
        token_type: oauth_response.token_type,
        expires_at_unix: now_unix_seconds().saturating_add(oauth_response.expires_in.unwrap_or(3600)),
    };

    *state.oauth_tokens.lock().await = Some(token.clone());
    Ok(Json(token))
}

async fn refresh_google_access_token(state: &ApplicationState, refresh_token: String) -> Result<OAuthToken, String> {
    let client_id = state.config.google_client_id.clone();
    let client_secret = state.config.google_client_secret.clone();

    if !state.config.has_google_oauth_config() {
        return Err("Google OAuth is not configured on the server.".to_string());
    }

    let payload = vec![
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
        ("refresh_token", refresh_token.as_str()),
        ("grant_type", "refresh_token"),
    ];

    let response = state.http_client
        .post("https://oauth2.googleapis.com/token")
        .form(&payload)
        .send()
        .await
        .map_err(|error| format!("token refresh failed: {error}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("token refresh returned {status}: {body}"));
    }

    let oauth_response: OAuthExchangeResponse = response.json().await.map_err(|error| format!("refresh response could not be parsed: {error}"))?;

    Ok(OAuthToken {
        access_token: oauth_response.access_token,
        refresh_token: Some(refresh_token),
        token_type: oauth_response.token_type,
        expires_at_unix: now_unix_seconds().saturating_add(oauth_response.expires_in.unwrap_or(3600)),
    })
}

fn now_unix_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

