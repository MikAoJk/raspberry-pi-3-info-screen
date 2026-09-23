mod log;

use std::{error,env, fs, path::Path as FilePath, sync::Arc, time::{SystemTime, UNIX_EPOCH}};

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{Html, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use chrono_tz::Europe::Oslo;
use ::log::{error, info};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::log::init_log4rs;

const WEATHER_CACHE_TTL: u64 = 15 * 60;
const CALENDAR_CACHE_TTL: u64 = 15 * 60;

#[tokio::main]
async fn main() -> Result<(), Box<dyn error::Error>> {
    init_log4rs();

    let application_state = ApplicationState::new();

    let app = Router::new()
        .route("/", get(root))
        .route("/oauth/callback", get(google_oauth_callback))
        .route("/api/oauth/google/config", get(get_google_oauth_config))
        .route("/api/oauth/google/token", post(store_google_token))
        .route("/api/oauth/google/refresh", post(refresh_google_token))
        .route("/api/slideshow", get(get_slideshow))
        .route("/api/slideshow/images/{filename}", get(get_slideshow_image))
        .route("/api/dashboard", get(get_dashboard))
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
    slideshow_directory: String,
    slideshow_interval_seconds: u64,
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
            slideshow_directory: env::var("SLIDESHOW_DIRECTORY").unwrap_or_else(|_| "static/slideshow".to_string()),
            slideshow_interval_seconds: env::var("SLIDESHOW_INTERVAL_SECONDS")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|seconds| *seconds > 0)
                .unwrap_or(30),
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
        is_cache_fresh(self.cached_at_unix, ttl_seconds)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CalendarCache {
    cached_at_unix: u64,
    payload: CalendarAggregation,
}

impl CalendarCache {
    fn is_fresh(&self, ttl_seconds: u64) -> bool {
        is_cache_fresh(self.cached_at_unix, ttl_seconds)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WeatherSnapshot {
    description: String,
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
    title: String,
    start: String,
    end: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DashboardResponse {
    weather: WeatherSnapshot,
    calendar: CalendarAggregation,
}

#[derive(Clone, Debug, Serialize)]
struct SlideshowResponse {
    images: Vec<String>,
    interval_seconds: u64,
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

fn string_to_static_str(s: String) -> &'static str {
    s.leak()
}

fn slideshow_content_type(filename: &str) -> Option<&'static str> {
    match FilePath::new(filename).extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "avif" => Some("image/avif"),
        "gif" => Some("image/gif"),
        "jpeg" | "jpg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

async fn get_slideshow(State(state): State<ApplicationState>) -> Result<Json<SlideshowResponse>, (StatusCode, String)> {
    let mut entries = match tokio::fs::read_dir(&state.config.slideshow_directory).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Json(SlideshowResponse {
                images: vec![],
                interval_seconds: state.config.slideshow_interval_seconds,
            }));
        }
        Err(error) => {
            error!("Unable to read slideshow directory: {error}");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "Unable to read slideshow directory.".to_string()));
        }
    };

    let mut images = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|error| {
        error!("Unable to read slideshow directory entry: {error}");
        (StatusCode::INTERNAL_SERVER_ERROR, "Unable to read slideshow directory.".to_string())
    })? {
        let file_type = entry.file_type().await.map_err(|error| {
            error!("Unable to inspect slideshow file: {error}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Unable to inspect slideshow directory.".to_string())
        })?;
        let Some(filename) = entry.file_name().to_str().map(str::to_string) else { continue; };

        if file_type.is_file() && slideshow_content_type(&filename).is_some() {
            images.push(filename);
        }
    }
    images.sort_by_key(|filename| filename.to_ascii_lowercase());

    Ok(Json(SlideshowResponse {
        images,
        interval_seconds: state.config.slideshow_interval_seconds,
    }))
}

async fn get_slideshow_image(
    State(state): State<ApplicationState>,
    Path(filename): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    if FilePath::new(&filename).file_name().and_then(|name| name.to_str()) != Some(filename.as_str()) {
        return Err((StatusCode::BAD_REQUEST, "Invalid slideshow filename.".to_string()));
    }
    let content_type = slideshow_content_type(&filename)
        .ok_or_else(|| (StatusCode::UNSUPPORTED_MEDIA_TYPE, "Unsupported slideshow image type.".to_string()))?;

    let directory = tokio::fs::canonicalize(&state.config.slideshow_directory)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Slideshow image not found.".to_string()))?;
    let path = tokio::fs::canonicalize(directory.join(&filename))
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Slideshow image not found.".to_string()))?;
    if !path.starts_with(&directory) {
        return Err((StatusCode::BAD_REQUEST, "Invalid slideshow filename.".to_string()));
    }

    let image = tokio::fs::read(path)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Slideshow image not found.".to_string()))?;
    Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "public, max-age=3600")
        .body(Body::from(image))
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, format!("Unable to build image response: {error}")))
}


async fn get_google_oauth_config(State(state): State<ApplicationState>) -> Result<Json<GoogleOAuthConfigResponse>, (StatusCode, String)> {
    if !state.config.has_google_oauth_config() {
        let mut missing_variables = Vec::new();

        if state.config.google_client_id.trim().is_empty() {
            missing_variables.push("GOOGLE_CLIENT_ID");
        }
        if state.config.google_client_secret.trim().is_empty() {
            missing_variables.push("GOOGLE_CLIENT_SECRET");
        }

        let missing_variables = missing_variables.join(", ");
        error!(
            "Google OAuth configuration is incomplete: missing {missing_variables}. \
             Configure the variable(s) in the service environment and restart the service. \
             redirect_uri={}",
            state.config.google_redirect_uri
        );
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "Google OAuth is unavailable because the server is missing {missing_variables}. \
                 Configure the variable(s) and restart the service."
            ),
        ));
    }

    let client_id = state.config.google_client_id.clone();
    let redirect_uri = state.config.google_redirect_uri.clone();
    info!("Providing Google OAuth configuration. redirect_uri={redirect_uri}");

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
        error!("Google OAuth denied: {error}");
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
    let display_start = Utc::now().with_timezone(&Oslo).date_naive();
    let start = Oslo.from_local_datetime(&display_start.and_hms_opt(0, 0, 0).unwrap()).single().unwrap().with_timezone(&Utc);
    let display_end = display_start + chrono::Duration::days(7);
    let end = Oslo.from_local_datetime(&display_end.and_hms_opt(0, 0, 0).unwrap()).single().unwrap().with_timezone(&Utc);
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
    let days = build_calendar_days(payload.items, display_start);
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

fn event_date(value: &GoogleEventTime) -> Option<NaiveDate> {
    value
        .date
        .as_deref()
        .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
        .or_else(|| {
            value
                .date_time
                .as_deref()
                .and_then(|date_time| DateTime::parse_from_rfc3339(date_time).ok())
                .map(|date_time| date_time.with_timezone(&Oslo).date_naive())
        })
}

fn event_date_span(item: &GoogleCalendarItem) -> Option<(NaiveDate, NaiveDate)> {
    let start = event_date(&item.start)?;
    let Some(end) = item.end.as_ref() else {
        return Some((start, start));
    };

    let inclusive_end = if item.start.date.is_some() {
        event_date(end).and_then(|date| date.pred_opt())
    } else {
        end.date_time
            .as_deref()
            .and_then(|date_time| DateTime::parse_from_rfc3339(date_time).ok())
            .and_then(|date_time| date_time.checked_sub_signed(chrono::Duration::nanoseconds(1)))
            .map(|date_time| date_time.with_timezone(&Oslo).date_naive())
    }
    .unwrap_or(start);

    Some((start, inclusive_end.max(start)))
}

fn build_calendar_days(items: Vec<GoogleCalendarItem>, display_start: NaiveDate) -> Vec<CalendarDay> {
    let mut events_by_day: std::collections::BTreeMap<String, Vec<CalendarEvent>> = std::collections::BTreeMap::new();
    let display_end = display_start + chrono::Duration::days(6);

    for item in items {
        let Some((event_start, event_end)) = event_date_span(&item) else { continue; };

        let start_text = if item.start.date_time.is_some() {
            item.start.date_time.clone().unwrap_or_default()
        } else {
            item.start.date.clone().unwrap_or_default()
        };

        let end_text = item.end.and_then(|value| value.date_time.or(value.date));
        let event = CalendarEvent {
            title: item.summary.unwrap_or_else(|| "Untitled event".to_string()),
            start: start_text,
            end: end_text,
        };

        let first_visible_date = event_start.max(display_start);
        let last_visible_date = event_end.min(display_end);
        if first_visible_date > last_visible_date {
            continue;
        }

        for offset in 0..=(last_visible_date - first_visible_date).num_days() {
            let date = first_visible_date + chrono::Duration::days(offset);
            events_by_day.entry(date.to_string()).or_default().push(event.clone());
        }
    }

    let mut days = Vec::new();
    for offset in 0..7 {
        let date = display_start + chrono::Duration::days(offset);
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
        .take(48)
        .map(|entry| HourlyWeather {
            time: entry.time,
            temperature_c: entry.data.instant.details.air_temperature,
            icon: resolve_weather_icon(entry.data.next_1_hours.as_ref().and_then(|summary| summary.summary.symbol_code.as_deref()).unwrap_or("clearsky_day")),
        })
        .collect();

    Ok(WeatherSnapshot {
        description: format!("Nå: {temperature}°C"),
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
        error!("Google OAuth tokens are not configured on the server.");
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

fn is_cache_fresh(cached_at_unix: u64, ttl_seconds: u64) -> bool {
    now_unix_seconds().saturating_sub(cached_at_unix) < ttl_seconds
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(title: &str, start: GoogleEventTime, end: GoogleEventTime) -> GoogleCalendarItem {
        GoogleCalendarItem {
            summary: Some(title.to_string()),
            start,
            end: Some(end),
        }
    }

    fn all_day(date: &str) -> GoogleEventTime {
        GoogleEventTime {
            date: Some(date.to_string()),
            date_time: None,
        }
    }

    fn timed(date_time: &str) -> GoogleEventTime {
        GoogleEventTime {
            date: None,
            date_time: Some(date_time.to_string()),
        }
    }

    #[test]
    fn accepts_only_browser_safe_slideshow_image_types() {
        assert_eq!(slideshow_content_type("photo.JPG"), Some("image/jpeg"));
        assert_eq!(slideshow_content_type("photo.webp"), Some("image/webp"));
        assert_eq!(slideshow_content_type("photo.svg"), None);
        assert_eq!(slideshow_content_type("photo.txt"), None);
    }

    #[test]
    fn displays_events_on_every_day_they_span() {
        let display_start = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        let days = build_calendar_days(
            vec![
                event("Holiday", all_day("2026-09-22"), all_day("2026-09-26")),
                event(
                    "Overnight",
                    timed("2026-09-23T23:00:00+02:00"),
                    timed("2026-09-24T01:00:00+02:00"),
                ),
            ],
            display_start,
        );

        assert_eq!(
            days.iter()
                .map(|day| day.events.iter().map(|event| event.title.as_str()).collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            vec![
                vec!["Holiday", "Overnight"],
                vec!["Holiday", "Overnight"],
                vec!["Holiday"],
                vec![],
                vec![],
                vec![],
                vec![],
            ]
        );
    }

    #[test]
    fn treats_google_end_dates_and_midnight_as_exclusive() {
        let display_start = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        let days = build_calendar_days(
            vec![
                event("All day", all_day("2026-09-23"), all_day("2026-09-24")),
                event(
                    "Until midnight",
                    timed("2026-09-23T20:00:00+02:00"),
                    timed("2026-09-24T00:00:00+02:00"),
                ),
            ],
            display_start,
        );

        assert_eq!(days[0].events.len(), 2);
        assert!(days[1].events.is_empty());
    }
}
