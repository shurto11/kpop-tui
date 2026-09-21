//! kpop-tui のブラウザ版サーバー。
//!
//! TUI版と同じ SQLite を読み取り専用で開き、JSON API + 静的アセットを配信する。
//! Tailscale 経由での公開を想定しているので、既定では 127.0.0.1 にのみバインドし、
//! `tailscale serve` にプロキシさせる。

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use kpop_tui::db::Database;
use kpop_tui::models::{ArtistData, Config, CreditData, TrackData, WriterData};

// ========== 静的アセット（バイナリに埋め込む） ==========

const INDEX_HTML: &str = include_str!("../../web/index.html");
const APP_CSS: &str = include_str!("../../web/app.css");
const APP_JS: &str = include_str!("../../web/app.js");

// ========== 状態 ==========

struct AppState {
    /// rusqlite の Connection は Sync ではないので Mutex で包む。
    /// 個人用途の読み取り専用アクセスなので直列化で十分。
    db: Mutex<Database>,
    /// Spotify oEmbed のサムネイルURLキャッシュ。
    /// 失敗（None）も一定時間は覚えておき、ネットワーク断のときに毎回待たされないようにする。
    art_cache: Mutex<HashMap<String, (Option<String>, Instant)>>,
}

type Shared = Arc<AppState>;

/// anyhow::Error を 500 JSON に変換するラッパー
struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({ "error": self.0.to_string() }));
        (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

type ApiResult<T> = std::result::Result<Json<T>, AppError>;

// ========== レスポンス型 ==========

#[derive(Serialize)]
struct DropItem {
    year: String,
    artist: String,
    track: String,
}

#[derive(Serialize)]
struct DropGroup {
    md: String,
    items: Vec<DropItem>,
}

/// 曲詳細（Home と 曲検索で共用）
#[derive(Serialize)]
struct SongDetail {
    artist: String,
    track: String,
    artist_label: Option<String>,
    track_data: Option<TrackData>,
    credits: Vec<CreditData>,
    around_day: Vec<DropGroup>,
    /// credits のうち WriterData に登録済みの名前（ハイライト用）
    known_writers: Vec<String>,
    art_url: Option<String>,
    /// 今日リリースの曲が無く、最新曲にフォールバックした場合 true
    is_random_fallback: bool,
    found: bool,
}

#[derive(Serialize)]
struct StatRow {
    role: String,
    count: i64,
    rank: i64,
}

#[derive(Serialize)]
struct YearCount {
    year: String,
    count: i64,
}

#[derive(Serialize)]
struct WriterResult {
    /// 検索語
    query: String,
    /// エイリアス解決後の代表名
    name: String,
    aliases: Vec<String>,
    writer_data: Option<WriterData>,
    stats: Vec<StatRow>,
    yearly: Vec<YearCount>,
    songs: Vec<CreditData>,
    found: bool,
}

// ========== クエリ型 ==========

#[derive(Deserialize)]
struct NameQuery {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct SongQuery {
    #[serde(default)]
    artist: String,
    #[serde(default)]
    track: String,
}

#[derive(Deserialize)]
struct FilterQuery {
    #[serde(default)]
    filter: String,
}

#[derive(Deserialize)]
struct SuggestQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    artist: String,
}

#[derive(Deserialize)]
struct UrlQuery {
    url: String,
}

// ========== main ==========

#[tokio::main]
async fn main() -> Result<()> {
    let config = load_config()?;
    let db_path = get_data_dir().join(&config.database.path);

    if !db_path.exists() {
        anyhow::bail!("Database not found: {}", db_path.display());
    }

    let db = Database::open_readonly(&db_path)
        .with_context(|| format!("Failed to open {}", db_path.display()))?;

    let state: Shared = Arc::new(AppState {
        db: Mutex::new(db),
        art_cache: Mutex::new(HashMap::new()),
    });

    let app = Router::new()
        .route("/", get(index))
        .route("/app.css", get(app_css))
        .route("/app.js", get(app_js))
        .route("/api/home", get(api_home))
        .route("/api/view/log", get(api_view_log))
        .route("/api/view/credits", get(api_view_credits))
        .route("/api/view/tracks", get(api_view_tracks))
        .route("/api/view/artists", get(api_view_artists))
        .route("/api/view/writers", get(api_view_writers))
        .route("/api/writer-names", get(api_writer_names))
        .route("/api/search/writer", get(api_search_writer))
        .route("/api/search/track", get(api_search_track))
        .route("/api/suggest/writers", get(api_suggest_writers))
        .route("/api/suggest/artists", get(api_suggest_artists))
        .route("/api/suggest/tracks", get(api_suggest_tracks))
        .route("/api/art", get(api_art))
        .layer(tower_http::compression::CompressionLayer::new())
        .with_state(state);

    let addr = listen_addr()?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind {}", addr))?;

    println!("kpop-web listening on http://{}", addr);
    println!("  db: {}", db_path.display());
    println!("  tailnet:  tailscale serve --bg {}", addr.port());

    axum::serve(listener, app).await?;
    Ok(())
}

/// バインド先。`--addr HOST:PORT` または環境変数 KPOP_WEB_ADDR、既定は 127.0.0.1:8787
fn listen_addr() -> Result<SocketAddr> {
    let args: Vec<String> = std::env::args().collect();
    let from_args = args
        .iter()
        .position(|a| a == "--addr")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let raw = from_args
        .or_else(|| std::env::var("KPOP_WEB_ADDR").ok())
        .unwrap_or_else(|| "127.0.0.1:8787".to_string());
    raw.parse()
        .with_context(|| format!("Invalid listen address: {}", raw))
}

fn get_data_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join("ssd").join("tui").join("kpop-tui"))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn load_config() -> Result<Config> {
    let path = get_data_dir().join("config.toml");
    if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        Ok(toml::from_str(&content)?)
    } else {
        Ok(Config::default())
    }
}

// ========== 静的アセット ==========

async fn index() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], INDEX_HTML)
}

async fn app_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], APP_CSS)
}

async fn app_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        APP_JS,
    )
}

// ========== View API ==========

async fn api_view_log(State(st): State<Shared>) -> ApiResult<Vec<CreditData>> {
    let db = st.db.lock().unwrap();
    Ok(Json(db.get_songs_by_log()?))
}

async fn api_view_credits(State(st): State<Shared>) -> ApiResult<Vec<CreditData>> {
    let db = st.db.lock().unwrap();
    Ok(Json(db.get_songs_sorted()?))
}

async fn api_view_tracks(
    State(st): State<Shared>,
    Query(q): Query<FilterQuery>,
) -> ApiResult<Vec<TrackData>> {
    let db = st.db.lock().unwrap();
    let tracks = match q.filter.as_str() {
        "soty" => db.get_soty()?,
        "aoty" => db.get_aoty()?,
        _ => db.get_all_track_data()?,
    };
    Ok(Json(tracks))
}

async fn api_view_artists(State(st): State<Shared>) -> ApiResult<Vec<ArtistData>> {
    let db = st.db.lock().unwrap();
    Ok(Json(db.get_artists_sorted()?))
}

async fn api_view_writers(State(st): State<Shared>) -> ApiResult<Vec<WriterData>> {
    let db = st.db.lock().unwrap();
    Ok(Json(db.get_writers()?))
}

/// WriterData に登録済みの名前一覧（一覧表示で太字にするため）
async fn api_writer_names(State(st): State<Shared>) -> ApiResult<Vec<String>> {
    let db = st.db.lock().unwrap();
    let mut names: Vec<String> = db.get_writer_data_names()?.into_iter().collect();
    names.sort();
    Ok(Json(names))
}

// ========== Home ==========

async fn api_home(State(st): State<Shared>) -> ApiResult<SongDetail> {
    let today = chrono::Local::now().format("%m-%d").to_string();

    let (picked, is_fallback) = {
        let db = st.db.lock().unwrap();
        let today_track = db.get_random_track_by_month_day(&today).unwrap_or(None);
        let is_fallback = today_track.is_none();
        let picked = match today_track {
            Some(t) => Some(t),
            None => db.get_newest_track().unwrap_or(None),
        };
        (picked, is_fallback)
    };

    let Some((artist, track)) = picked else {
        return Ok(Json(empty_detail(String::new(), String::new())));
    };

    let mut detail = build_song_detail(&st, &artist, &track).await?;
    detail.is_random_fallback = is_fallback;
    Ok(Json(detail))
}

// ========== Search ==========

async fn api_search_track(
    State(st): State<Shared>,
    Query(q): Query<SongQuery>,
) -> ApiResult<SongDetail> {
    if q.artist.is_empty() || q.track.is_empty() {
        return Ok(Json(empty_detail(q.artist, q.track)));
    }
    Ok(Json(build_song_detail(&st, &q.artist, &q.track).await?))
}

async fn api_search_writer(
    State(st): State<Shared>,
    Query(q): Query<NameQuery>,
) -> ApiResult<WriterResult> {
    let db = st.db.lock().unwrap();
    let query = q.name.clone();

    if query.is_empty() {
        return Ok(Json(WriterResult {
            query,
            name: String::new(),
            aliases: Vec::new(),
            writer_data: None,
            stats: Vec::new(),
            yearly: Vec::new(),
            songs: Vec::new(),
            found: false,
        }));
    }

    let primary = db.get_primary_name(&query)?;
    let aliases = db.get_aliases_for_writer(&primary).unwrap_or_default();
    let songs = db.search_songs_by_writer(&query)?;
    let writer_data = db.get_writer(&primary).unwrap_or(None);

    // 統計（TUIの Statistics ブロックと同じ並び）
    let role_counts: HashMap<String, i64> = db.get_writer_stats(&query)?.into_iter().collect();
    let ranks: HashMap<String, i64> = db.get_writer_ranks(&query)?.into_iter().collect();
    let total = db.get_writer_total_count(&query)?;
    let aoty = db.get_writer_aoty_count(&query)?;
    let soty = db.get_writer_soty_count(&query)?;

    let stats = vec![
        stat_row("lyricist", role_counts.get("lyricist").copied().unwrap_or(0), &ranks),
        stat_row("composer", role_counts.get("composer").copied().unwrap_or(0), &ranks),
        stat_row("arranger", role_counts.get("arranger").copied().unwrap_or(0), &ranks),
        stat_row("writer", role_counts.get("writer").copied().unwrap_or(0), &ranks),
        stat_row("total", total, &ranks),
        stat_row("AOTY", aoty, &ranks),
        stat_row("SOTY", soty, &ranks),
    ];

    let yearly: Vec<YearCount> = db
        .get_writer_yearly_stats(&query)?
        .into_iter()
        .map(|(year, count)| YearCount { year, count })
        .collect();

    let found = !songs.is_empty() || writer_data.is_some();

    Ok(Json(WriterResult {
        query,
        name: primary,
        aliases,
        writer_data,
        stats,
        yearly,
        songs,
        found,
    }))
}

fn stat_row(role: &str, count: i64, ranks: &HashMap<String, i64>) -> StatRow {
    StatRow {
        role: role.to_string(),
        count,
        rank: ranks.get(role).copied().unwrap_or(0),
    }
}

// ========== 補完 ==========

async fn api_suggest_writers(
    State(st): State<Shared>,
    Query(q): Query<SuggestQuery>,
) -> ApiResult<Vec<String>> {
    let db = st.db.lock().unwrap();
    Ok(Json(db.get_writer_suggestions(&q.q)?))
}

async fn api_suggest_artists(
    State(st): State<Shared>,
    Query(q): Query<SuggestQuery>,
) -> ApiResult<Vec<String>> {
    let db = st.db.lock().unwrap();
    let needle = q.q.to_lowercase();
    let artists: Vec<String> = db
        .get_all_artists()?
        .into_iter()
        .filter(|a| needle.is_empty() || a.to_lowercase().contains(&needle))
        .take(30)
        .collect();
    Ok(Json(artists))
}

async fn api_suggest_tracks(
    State(st): State<Shared>,
    Query(q): Query<SuggestQuery>,
) -> ApiResult<Vec<String>> {
    let db = st.db.lock().unwrap();
    if q.artist.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let needle = q.q.to_lowercase();
    let tracks: Vec<String> = db
        .get_tracks_by_artist(&q.artist)?
        .into_iter()
        .filter(|t| needle.is_empty() || t.to_lowercase().contains(&needle))
        .collect();
    Ok(Json(tracks))
}

// ========== アルバムアート ==========

async fn api_art(
    State(st): State<Shared>,
    Query(q): Query<UrlQuery>,
) -> ApiResult<serde_json::Value> {
    let url = art_url_for(&st, &q.url).await;
    Ok(Json(serde_json::json!({ "art_url": url })))
}

/// 取得失敗をキャッシュしておく時間
const ART_FAILURE_TTL: Duration = Duration::from_secs(300);

/// Spotify URL からサムネイルURLを引く。成功は恒久キャッシュ、失敗は5分だけキャッシュ。
async fn art_url_for(st: &Shared, spotify_url: &str) -> Option<String> {
    if spotify_url.is_empty() {
        return None;
    }
    if let Some((cached, at)) = st.art_cache.lock().unwrap().get(spotify_url) {
        if cached.is_some() || at.elapsed() < ART_FAILURE_TTL {
            return cached.clone();
        }
    }

    // reqwest::blocking は tokio ワーカースレッド上で呼べないので専用スレッドに逃がす
    let target = spotify_url.to_string();
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(kpop_tui::scraper::fetch_album_art_url(&target).ok());
    });
    let result = rx.await.unwrap_or(None);

    st.art_cache
        .lock()
        .unwrap()
        .insert(spotify_url.to_string(), (result.clone(), Instant::now()));
    result
}

// ========== 曲詳細の組み立て ==========

fn empty_detail(artist: String, track: String) -> SongDetail {
    SongDetail {
        artist,
        track,
        artist_label: None,
        track_data: None,
        credits: Vec::new(),
        around_day: Vec::new(),
        known_writers: Vec::new(),
        art_url: None,
        is_random_fallback: false,
        found: false,
    }
}

async fn build_song_detail(st: &Shared, artist: &str, track: &str) -> Result<SongDetail> {
    // DBロックはこのブロック内で閉じる（await をまたがせない）
    let (credits, track_data, artist_label, around_day, known_writers) = {
        let db = st.db.lock().unwrap();
        let credits = db.search_song(artist, track)?;
        let track_data = db.get_song_add(artist, track).unwrap_or(None);
        let artist_label = db.get_artist(artist).ok().flatten().and_then(|a| a.label);
        let around_day = around_day_groups(&db, artist, track, credits.first());

        let registered: HashSet<String> = db.get_writer_data_names().unwrap_or_default();
        let known_writers: Vec<String> = credits
            .iter()
            .filter_map(|c| c.name.clone())
            .filter(|n| registered.contains(n))
            .collect::<HashSet<String>>()
            .into_iter()
            .collect();

        (credits, track_data, artist_label, around_day, known_writers)
    };

    let art_url = match track_data.as_ref().and_then(|t| t.spotify.as_deref()) {
        Some(url) if !url.is_empty() => art_url_for(st, url).await,
        _ => None,
    };

    let found = !credits.is_empty() || track_data.is_some();

    Ok(SongDetail {
        artist: artist.to_string(),
        track: track.to_string(),
        artist_label,
        track_data,
        credits,
        around_day,
        known_writers,
        art_url,
        is_random_fallback: false,
        found,
    })
}

/// リリース日の前日・当日・翌日に出た他の Title/Pre 曲（TUIの Around-The-Day Drops と同じ）
fn around_day_groups(
    db: &Database,
    artist: &str,
    track: &str,
    credit: Option<&CreditData>,
) -> Vec<DropGroup> {
    let Some(date_str) = credit.and_then(|c| c.date.as_deref()) else {
        return Vec::new();
    };
    let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") else {
        return Vec::new();
    };

    let prev = date - chrono::Duration::days(1);
    let next = date + chrono::Duration::days(1);
    let mds = [
        prev.format("%m-%d").to_string(),
        date.format("%m-%d").to_string(),
        next.format("%m-%d").to_string(),
    ];
    let album = credit.and_then(|c| c.album.as_deref()).unwrap_or("");

    let Ok(drops) = db.get_around_day_drops(&mds[0], &mds[1], &mds[2], artist, track, album) else {
        return Vec::new();
    };

    mds.iter()
        .map(|md| DropGroup {
            md: md.clone(),
            items: drops
                .iter()
                .filter(|(d, _, _)| d.len() >= 10 && &d[5..] == md.as_str())
                .map(|(d, a, t)| DropItem {
                    year: d[..4].to_string(),
                    artist: a.clone(),
                    track: t.clone(),
                })
                .collect(),
        })
        .collect()
}
