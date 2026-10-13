//! 曲ごとの再生回数（ViewTrackData の Plays 列）。
//!
//! 過去分は Spotify のデータダウンロード（拡張ストリーミング再生履歴）、それ以降は Last.fm の
//! scrobble から数える。両方に同じ再生が入る期間があるので、データダウンロードの最後の再生より
//! 後の Last.fm だけを数える。
//!
//! 生の再生ログは kpop.db ではなく plays.db に置く。全期間の履歴は大きく、終了時の SQL ダンプ
//! （kpop-tui-data への push）に毎回載ってしまうため。データダウンロードと Last.fm から
//! 作り直せる派生データなので、バックアップはしない。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::{params, Connection};

use crate::models::{LastfmConfig, TrackData};

/// データダウンロードで1回と数える最短の再生時間。Spotify 自身が1再生と数えるのと同じ30秒
const MIN_MS_PLAYED: i64 = 30_000;

const SOURCE_SPOTIFY: &str = "spotify";
const SOURCE_LASTFM: &str = "lastfm";

const LASTFM_API: &str = "https://ws.audioscrobbler.com/2.0/";
/// getRecentTracks の1ページの上限
const LASTFM_PAGE_LIMIT: u32 = 200;
/// Last.fm に連打しない
const LASTFM_INTERVAL: Duration = Duration::from_millis(250);

/// 再生1回分
#[derive(Debug, Clone, PartialEq)]
pub struct Play {
    /// 再生を始めた時刻（UNIX秒）。Last.fm の scrobble 時刻に合わせて開始時刻で持つ
    pub played_at: i64,
    pub artist: String,
    pub track: String,
    /// Spotify のトラックID。Last.fm 由来は名前しかないので None
    pub spotify_id: Option<String>,
    /// 聴いた時間（ミリ秒）。Last.fm 由来は None（scrobble された時点で1回）
    pub ms_played: Option<i64>,
}

impl Play {
    /// 1回と数えるか。データダウンロードの30秒未満の再生は数えず、名前の対応を覚えるのにだけ使う
    pub fn is_counted(&self) -> bool {
        self.ms_played.is_none_or(|ms| ms >= MIN_MS_PLAYED)
    }
}

/// TrackData の (artist, track)
pub type TrackKey = (String, String);

pub struct PlayDb {
    conn: Connection,
}

impl PlayDb {
    /// 再生ログのDBを開く（なければ作成）
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path).context("Failed to open plays database")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS play_log (
                source TEXT NOT NULL,
                played_at INTEGER NOT NULL,
                artist TEXT NOT NULL,
                track TEXT NOT NULL,
                spotify_id TEXT,
                ms_played INTEGER,
                UNIQUE(source, played_at, artist, track)
            );",
        )?;
        Ok(Self { conn })
    }

    /// データダウンロードの再生を追加する。取り込み済みの再生は飛ばす。戻り値は新しく入った件数
    pub fn insert_spotify(&self, plays: &[Play]) -> Result<usize> {
        self.insert(SOURCE_SPOTIFY, plays)
    }

    /// Last.fm の scrobble を追加する。取り込み済みの再生は飛ばす。戻り値は新しく入った件数
    pub fn insert_lastfm(&self, plays: &[Play]) -> Result<usize> {
        self.insert(SOURCE_LASTFM, plays)
    }

    fn insert(&self, source: &str, plays: &[Play]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let mut inserted = 0;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO play_log (source, played_at, artist, track, spotify_id, ms_played)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for p in plays {
                inserted += stmt.execute(params![
                    source,
                    p.played_at,
                    p.artist,
                    p.track,
                    p.spotify_id,
                    p.ms_played
                ])?;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// 取り込み済みの Last.fm の最新時刻。次の同期はここから取りに行く
    pub fn last_lastfm_at(&self) -> Result<Option<i64>> {
        Ok(self.conn.query_row(
            "SELECT MAX(played_at) FROM play_log WHERE source = ?1",
            params![SOURCE_LASTFM],
            |row| row.get(0),
        )?)
    }

    /// TrackData の (artist, track) ごとの再生回数。再生の無い曲は入らない
    pub fn play_counts(&self, tracks: &[TrackData]) -> Result<HashMap<TrackKey, i64>> {
        Ok(Matcher::new(tracks).count(&self.groups()?))
    }

    /// 再生を (artist, track, spotify_id) ごとにまとめる。count は数える再生（Play::is_counted）の数。
    /// Last.fm はデータダウンロードの最後の再生より後だけ（それ以前はデータダウンロードに入っている）
    fn groups(&self) -> Result<Vec<PlayGroup>> {
        let mut stmt = self.conn.prepare(
            "SELECT artist, track, spotify_id,
                    COUNT(CASE WHEN ms_played IS NULL OR ms_played >= ?3 THEN 1 END)
             FROM play_log
             WHERE source = ?1
                OR (source = ?2 AND played_at > COALESCE(
                       (SELECT MAX(played_at) FROM play_log WHERE source = ?1), -1))
             GROUP BY artist, track, spotify_id",
        )?;
        let rows = stmt.query_map(params![SOURCE_SPOTIFY, SOURCE_LASTFM, MIN_MS_PLAYED], |row| {
            Ok(PlayGroup {
                artist: row.get(0)?,
                track: row.get(1)?,
                spotify_id: row.get(2)?,
                count: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

/// 同じ名前・IDの再生をまとめたもの
#[derive(Debug, Clone)]
struct PlayGroup {
    artist: String,
    track: String,
    spotify_id: Option<String>,
    /// 数える再生の数。30秒未満しか無ければ 0（名前の対応を覚えるのにだけ使う）
    count: i64,
}

// ========== 再生と TrackData の突き合わせ ==========

/// "(feat. X)" "[feat. X]" "(Prod. X)" と末尾の " feat. X"。表記が揺れるので外して比べる
static FEAT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*[\(\[]\s*(feat|ft|prod|with)\b[^\)\]]*[\)\]]|\s+(feat|ft)\.\s.*$").unwrap()
});
/// "JIKJIN - KR ver.-" のような韓国語版の印。DB は韓国語版しか持たない
static KR_VER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s+-\s+(kr|kor|korean)\.?\s*ver.*$").unwrap());
/// 括弧内の末尾の "Ver."。Genius は "(Baila Ver.)"、Spotify は "(Baila)" と書くことがある
static VER_SUFFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s+ver(?:\.|sion)?\s*([\)\]])").unwrap());
static PAREN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\(\[]([^\)\]]*)[\)\]]").unwrap());
/// "(Remix)" "(Inst.)" のような版違いの印。これが付く曲は括弧を外して比べない（別の曲として数える）
static VERSION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(remix|mix|inst|instrumental|sped|slowed|acoustic|live|edit|unplugged|ver|version|english|eng|japanese|jp|remaster|remastered|cover|demo|acapella)\b",
    )
    .unwrap()
});
/// 連名の区切り。Spotify は "aespa, Ty Dolla $ign"、Genius は "NMIXX & Anderson .Paak"
static ARTIST_SPLIT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s*(?:&|,|×|\s+x\s+|\s+with\s+)\s*").unwrap());

/// 比較用に潰す。大文字小文字・空白・記号の違いを無視する
fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 曲名の比較用の候補。
/// Genius は "여우비 (Crush)"、Spotify は "Crush" や "Pocket (YEJI)" のように括弧の付け方が違うので、
/// 括弧の外・中もそれぞれ候補にする
fn title_variants(title: &str) -> HashSet<String> {
    let title = FEAT_RE.replace_all(title, "");
    let title = KR_VER_RE.replace(&title, "");

    let mut out = HashSet::from([
        squash(&title),
        squash(&VER_SUFFIX_RE.replace_all(&title, "$1")),
    ]);
    let inners: Vec<&str> = PAREN_RE
        .captures_iter(&title)
        .filter_map(|c| c.get(1).map(|m| m.as_str()))
        .collect();
    if !inners.iter().any(|s| VERSION_RE.is_match(s)) {
        out.insert(squash(&PAREN_RE.replace_all(&title, "")));
        out.extend(inners.iter().map(|s| squash(s)));
    }
    out.remove("");
    out
}

/// アーティスト名の比較用の候補。
/// "YEJI & ITZY" "V8 (VERNON & THE 8)" "KEY (SHINee)" のような連名・括弧書きを、1人ずつにも分ける
fn artist_variants(artist: &str) -> HashSet<String> {
    let mut pieces = vec![artist.to_string(), PAREN_RE.replace_all(artist, " ").into_owned()];
    pieces.extend(
        PAREN_RE
            .captures_iter(artist)
            .filter_map(|c| c.get(1).map(|m| m.as_str().to_string())),
    );

    let mut out = HashSet::new();
    for piece in &pieces {
        out.insert(squash(piece));
        out.extend(ARTIST_SPLIT_RE.split(piece).map(squash));
    }
    out.remove("");
    out
}

/// Spotify URL（https://open.spotify.com/track/<id>?si=...）からトラックIDを取り出す
fn spotify_track_id(url: &str) -> Option<&str> {
    let id = url.split("/track/").nth(1)?.split(['?', '/']).next()?;
    (!id.is_empty()).then_some(id)
}

struct Matcher {
    /// Spotify のトラックID → TrackData。BPM 取得の取り違えで、別の曲の行に同じ URL が入っていることがある
    by_id: HashMap<String, Vec<TrackKey>>,
    /// (アーティスト候補, 曲名候補) → TrackData
    by_name: HashMap<(String, String), Vec<TrackKey>>,
}

impl Matcher {
    fn new(tracks: &[TrackData]) -> Self {
        let mut by_id: HashMap<String, Vec<TrackKey>> = HashMap::new();
        let mut by_name: HashMap<(String, String), Vec<TrackKey>> = HashMap::new();
        for t in tracks {
            let key = (t.artist.clone(), t.track.clone());
            if let Some(id) = t.spotify.as_deref().and_then(spotify_track_id) {
                by_id.entry(id.to_string()).or_default().push(key.clone());
            }
            let titles = title_variants(&t.track);
            for a in artist_variants(&t.artist) {
                for title in &titles {
                    let keys = by_name.entry((a.clone(), title.clone())).or_default();
                    if !keys.contains(&key) {
                        keys.push(key.clone());
                    }
                }
            }
        }
        Self { by_id, by_name }
    }

    fn count(&self, groups: &[PlayGroup]) -> HashMap<TrackKey, i64> {
        // IDで決まった再生から、Spotify 側の名前 → TrackData の対応を覚える。
        // ソロ曲がグループ名義で出ている（LEESEO "Super ICY" → IVE "Super ICY (LEESEO Solo)"）など
        // 名前では合わない曲も、Last.fm（名前しか無い）や別IDの同じ曲で数えられるようにするため。
        // 連名の曲は先頭の名義になる（KISS OF LIFE "Superpower" → VALORANT "SUPERPOWER"）。
        // DB の ID では30秒未満しか聴いていないこともあるので、数えない再生からも覚える
        let by_id: Vec<Vec<&TrackKey>> = groups.iter().map(|g| self.lookup_by_id(g)).collect();
        let mut learned: HashMap<(String, String), &[&TrackKey]> = HashMap::new();
        for (g, keys) in groups.iter().zip(&by_id) {
            if !keys.is_empty() {
                learned.insert((squash(&g.artist), squash(&g.track)), keys);
            }
        }

        let mut counts: HashMap<TrackKey, i64> = HashMap::new();
        for (g, id_keys) in groups.iter().zip(&by_id) {
            if g.count == 0 {
                continue;
            }
            let keys: Vec<&TrackKey> = if !id_keys.is_empty() {
                id_keys.clone()
            } else if let Some(keys) = learned.get(&(squash(&g.artist), squash(&g.track))) {
                keys.to_vec()
            } else {
                self.lookup_by_name(&g.artist, &g.track)
            };
            for key in keys {
                *counts.entry(key.clone()).or_default() += g.count;
            }
        }
        counts
    }

    /// IDで引く。同じ URL が複数の行に入っているときは、名前も合う行に絞る
    /// （合う行が無ければ、同じ曲が別名義で2行あるとみなして全部）
    fn lookup_by_id(&self, g: &PlayGroup) -> Vec<&TrackKey> {
        let Some(keys) = g.spotify_id.as_ref().and_then(|id| self.by_id.get(id)) else {
            return Vec::new();
        };
        if keys.len() == 1 {
            return keys.iter().collect();
        }
        let by_name = self.lookup_by_name(&g.artist, &g.track);
        let narrowed: Vec<&TrackKey> = keys.iter().filter(|k| by_name.contains(k)).collect();
        if narrowed.is_empty() {
            keys.iter().collect()
        } else {
            narrowed
        }
    }

    /// 名前で引く。同じ曲が DB に別名義で2行あるときは両方に数える
    fn lookup_by_name(&self, artist: &str, track: &str) -> Vec<&TrackKey> {
        let titles = title_variants(track);
        let mut out: Vec<&TrackKey> = Vec::new();
        for a in artist_variants(artist) {
            for title in &titles {
                for key in self.by_name.get(&(a.clone(), title.clone())).into_iter().flatten() {
                    if !out.contains(&key) {
                        out.push(key);
                    }
                }
            }
        }
        out
    }
}

// ========== Spotify のデータダウンロード ==========

/// データダウンロードを読んだ結果
pub struct SpotifyHistory {
    /// 曲の再生。30秒未満も含む（Play::is_counted で数えるものを選ぶ）
    pub plays: Vec<Play>,
    pub files: usize,
    /// 読んだ行数（ポッドキャスト・30秒未満も含む）
    pub rows: usize,
}

/// Spotify のデータダウンロード（拡張ストリーミング再生履歴）を読む。
/// path はフォルダ（中の Streaming_History_Audio_*.json をすべて読む）か、JSON ファイル1つ
pub fn read_spotify_history(path: &Path) -> Result<SpotifyHistory> {
    let files = if path.is_dir() {
        let mut files = Vec::new();
        collect_history_files(path, &mut files)?;
        files.sort();
        files
    } else {
        vec![path.to_path_buf()]
    };
    if files.is_empty() {
        anyhow::bail!("No Streaming_History_Audio_*.json in {}", path.display());
    }

    let mut history = SpotifyHistory { plays: Vec::new(), files: files.len(), rows: 0 };
    for file in &files {
        let body = std::fs::read_to_string(file)
            .with_context(|| format!("Failed to read {}", file.display()))?;
        let rows: Vec<serde_json::Value> = serde_json::from_str(&body)
            .with_context(|| format!("Failed to parse {}", file.display()))?;
        history.rows += rows.len();
        history.plays.extend(rows.iter().filter_map(parse_history_row));
    }
    Ok(history)
}

fn collect_history_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("Failed to read {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect_history_files(&path, out)?;
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with("Streaming_History_Audio") && name.ends_with(".json") {
            out.push(path);
        }
    }
    Ok(())
}

/// 1行を Play に変換する。ポッドキャスト（曲名が null）は飛ばす
fn parse_history_row(row: &serde_json::Value) -> Option<Play> {
    let ms_played = row["ms_played"].as_i64()?;
    let track = row["master_metadata_track_name"].as_str()?;
    let artist = row["master_metadata_album_artist_name"].as_str()?;
    // ts は再生が終わった時刻
    let ended = chrono::DateTime::parse_from_rfc3339(row["ts"].as_str()?).ok()?;

    Some(Play {
        played_at: ended.timestamp() - ms_played / 1000,
        artist: artist.to_string(),
        track: track.to_string(),
        spotify_id: row["spotify_track_uri"]
            .as_str()
            .and_then(|uri| uri.strip_prefix("spotify:track:"))
            .map(|id| id.to_string()),
        ms_played: Some(ms_played),
    })
}

// ========== Last.fm ==========

/// Last.fm の scrobble を取る。from（UNIX秒）より後だけ、None なら全件
pub fn fetch_lastfm(config: &LastfmConfig, from: Option<i64>) -> Result<Vec<Play>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("kpop-tui")
        .build()?;

    let mut out = Vec::new();
    let mut page = 1;
    loop {
        let mut query = vec![
            ("method", "user.getrecenttracks".to_string()),
            ("user", config.user.clone()),
            ("api_key", config.api_key.clone()),
            ("format", "json".to_string()),
            ("limit", LASTFM_PAGE_LIMIT.to_string()),
            ("page", page.to_string()),
        ];
        if let Some(from) = from {
            query.push(("from", from.to_string()));
        }

        // reqwest のエラーは URL（api_key 入り）を含むので外す
        let body = client
            .get(LASTFM_API)
            .query(&query)
            .send()
            .and_then(|r| r.text())
            .map_err(|e| anyhow::anyhow!("Network error: {}", e.without_url()))?;
        let json: serde_json::Value =
            serde_json::from_str(&body).context("Failed to parse Last.fm response")?;

        let (plays, total_pages) = parse_recent_tracks(&json)?;
        out.extend(plays);
        if page >= total_pages {
            break;
        }
        page += 1;
        std::thread::sleep(LASTFM_INTERVAL);
    }
    Ok(out)
}

/// user.getRecentTracks の1ページを読む。戻り値は (scrobble, 総ページ数)
fn parse_recent_tracks(json: &serde_json::Value) -> Result<(Vec<Play>, u32)> {
    if let Some(code) = json.get("error") {
        anyhow::bail!("Last.fm error {}: {}", code, json["message"].as_str().unwrap_or(""));
    }
    let recent = json.get("recenttracks").context("Unexpected Last.fm response")?;
    let total_pages = recent["@attr"]["totalPages"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    // 1件だけのときは配列ではなくオブジェクトで返ってくる
    let tracks: Vec<&serde_json::Value> = match &recent["track"] {
        serde_json::Value::Array(a) => a.iter().collect(),
        v @ serde_json::Value::Object(_) => vec![v],
        _ => Vec::new(),
    };

    let plays = tracks
        .into_iter()
        .filter_map(|t| {
            // 再生中の曲は時刻が無い。聴き終わって scrobble されてから数える
            let played_at = t["date"]["uts"].as_str()?.parse().ok()?;
            Some(Play {
                played_at,
                artist: t["artist"]["#text"].as_str()?.to_string(),
                track: t["name"].as_str()?.to_string(),
                spotify_id: None,
                ms_played: None,
            })
        })
        .collect();
    Ok((plays, total_pages))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(artist: &str, title: &str, spotify: Option<&str>) -> TrackData {
        let mut t = TrackData::new(title.to_string(), artist.to_string());
        t.spotify = spotify.map(|s| s.to_string());
        t
    }

    /// Last.fm の scrobble
    fn scrobble(at: i64, artist: &str, title: &str) -> Play {
        Play { played_at: at, artist: artist.to_string(), track: title.to_string(), spotify_id: None, ms_played: None }
    }

    /// データダウンロードの再生
    fn listen(at: i64, artist: &str, title: &str, id: &str, ms: i64) -> Play {
        Play { spotify_id: Some(id.to_string()), ms_played: Some(ms), ..scrobble(at, artist, title) }
    }

    fn key(artist: &str, title: &str) -> TrackKey {
        (artist.to_string(), title.to_string())
    }

    #[test]
    fn matches_spotify_names_to_genius_names() {
        let m = Matcher::new(&[
            track("xikers", "문제아 (Outsider)", None),
            track("ITZY", "Pocket", None),
            track("YEJI & ITZY", "Pocket", None),
            track("NMIXX & Anderson\u{a0}.Paak", "Caution", None),
            track("aespa", "WDA (Whole Different Animal)", None),
            track("V8\u{a0}(VERNON & THE 8)", "Song", None),
            track("ZICO", "사랑해 (I LOVE YOU)", None),
            track("NMIXX", "O.O Part 1 (Baila Ver.)", None),
        ]);
        let hit = |a: &str, t: &str| m.lookup_by_name(a, t).into_iter().cloned().collect::<Vec<_>>();

        assert_eq!(hit("xikers", "Outsider"), vec![key("xikers", "문제아 (Outsider)")]);
        assert_eq!(hit("NMIXX, Anderson .Paak", "Caution"), vec![key("NMIXX & Anderson\u{a0}.Paak", "Caution")]);
        assert_eq!(
            hit("aespa", "WDA (Whole Different Animal) [feat. G-DRAGON]"),
            vec![key("aespa", "WDA (Whole Different Animal)")]
        );
        assert_eq!(hit("THE 8, Vernon", "Song"), vec![key("V8\u{a0}(VERNON & THE 8)", "Song")]);
        assert_eq!(hit("ZICO", "I LOVE YOU - KR Ver."), vec![key("ZICO", "사랑해 (I LOVE YOU)")]);
        assert_eq!(hit("NMIXX", "O.O Part 1 (Baila)"), vec![key("NMIXX", "O.O Part 1 (Baila Ver.)")]);
        // 同じ曲が別名義で2行あるときは両方
        let mut both = hit("ITZY", "Pocket (YEJI)");
        both.sort();
        assert_eq!(both, vec![key("ITZY", "Pocket"), key("YEJI & ITZY", "Pocket")]);
    }

    #[test]
    fn remixes_are_different_songs() {
        let m = Matcher::new(&[track("Billlie", "ZAP", None), track("ITZY", "Motto", None)]);
        assert!(m.lookup_by_name("Billlie", "ZAP - ultraviolet remix").is_empty());
        assert!(m.lookup_by_name("Billlie", "ZAP (Remix)").is_empty());
        assert!(m.lookup_by_name("ITZY", "Motto - Unplugged Ver.").is_empty());
        assert!(m.lookup_by_name("ITZY", "Motto (Band Ver.)").is_empty());
        // アーティストが違えば同じ曲名でも数えない
        assert!(m.lookup_by_name("Other", "ZAP").is_empty());
    }

    #[test]
    fn learns_names_from_id_matches() {
        let m = Matcher::new(&[track("LEESEO", "Super ICY", Some("https://open.spotify.com/track/ID1?si=x"))]);
        let groups = vec![
            // データダウンロード（IDあり）で、Spotify 側の名前を覚える
            PlayGroup { artist: "IVE".into(), track: "Super ICY (LEESEO Solo)".into(), spotify_id: Some("ID1".into()), count: 3 },
            // Last.fm は名前しか無い
            PlayGroup { artist: "IVE".into(), track: "Super ICY (LEESEO Solo)".into(), spotify_id: None, count: 2 },
        ];
        assert_eq!(m.count(&groups), HashMap::from([(key("LEESEO", "Super ICY"), 5)]));
    }

    #[test]
    fn shared_spotify_url_is_narrowed_by_name() {
        // BPM 取得の取り違えで "like JENNIE" の行に ExtraL の URL が入っている
        let m = Matcher::new(&[
            track("JENNIE", "ExtraL", Some("https://open.spotify.com/track/EXTRAL")),
            track("JENNIE", "like JENNIE", Some("https://open.spotify.com/track/EXTRAL")),
            track("ITZY", "Pocket", Some("https://open.spotify.com/track/POCKET")),
            track("YEJI & ITZY", "Pocket", Some("https://open.spotify.com/track/POCKET")),
        ]);
        let groups = vec![
            PlayGroup { artist: "JENNIE".into(), track: "ExtraL (feat. Doechii)".into(), spotify_id: Some("EXTRAL".into()), count: 5 },
            PlayGroup { artist: "JENNIE".into(), track: "like JENNIE".into(), spotify_id: Some("LIKE".into()), count: 3 },
            // 名前ではどちらにも合わないときは、同じ曲が別名義で2行あるとみなして両方
            PlayGroup { artist: "YEJI".into(), track: "Pocket - YEJI Solo".into(), spotify_id: Some("POCKET".into()), count: 2 },
        ];
        assert_eq!(
            m.count(&groups),
            HashMap::from([
                (key("JENNIE", "ExtraL"), 5),
                (key("JENNIE", "like JENNIE"), 3),
                (key("ITZY", "Pocket"), 2),
                (key("YEJI & ITZY", "Pocket"), 2),
            ])
        );
    }

    #[test]
    fn lastfm_counts_only_after_spotify_history() {
        let db = PlayDb::open(":memory:").unwrap();
        let tracks = [track("NMIXX", "LOUD", Some("https://open.spotify.com/track/LOUD"))];

        db.insert_lastfm(&[scrobble(50, "NMIXX", "LOUD"), scrobble(150, "NMIXX", "LOUD")]).unwrap();
        // データダウンロードが無いうちは Last.fm を全部数える
        assert_eq!(db.play_counts(&tracks).unwrap()[&key("NMIXX", "LOUD")], 2);

        // 100 までデータダウンロードがあるなら、Last.fm は 150 の1回だけ
        db.insert_spotify(&[listen(10, "NMIXX", "LOUD", "LOUD", 200_000), listen(100, "NMIXX", "LOUD", "LOUD", 200_000)]).unwrap();
        assert_eq!(db.play_counts(&tracks).unwrap()[&key("NMIXX", "LOUD")], 3);

        // 同じ再生をもう一度入れても増えない
        assert_eq!(db.insert_spotify(&[listen(10, "NMIXX", "LOUD", "LOUD", 200_000)]).unwrap(), 0);
        assert_eq!(db.last_lastfm_at().unwrap(), Some(150));
    }

    #[test]
    fn learns_names_from_short_plays() {
        // 連名の曲は先頭の名義（VALORANT）で出る。DB の ID では30秒未満しか聴いていない
        let db = PlayDb::open(":memory:").unwrap();
        let tracks = [
            track("KISS OF LIFE", "Superpower", Some("https://open.spotify.com/track/DB")),
            track("NMIXX", "LOUD", Some("https://open.spotify.com/track/LOUD")),
        ];
        db.insert_spotify(&[
            listen(10, "VALORANT", "SUPERPOWER", "DB", 5_000),
            listen(20, "VALORANT", "SUPERPOWER", "SINGLE", 200_000),
            listen(30, "VALORANT", "SUPERPOWER", "ALBUM", 200_000),
            listen(40, "NMIXX", "LOUD", "LOUD", 5_000),
        ])
        .unwrap();
        // 30秒未満しか無い曲は空欄のまま（0 と出さない）
        assert_eq!(
            db.play_counts(&tracks).unwrap(),
            HashMap::from([(key("KISS OF LIFE", "Superpower"), 2)])
        );
    }

    #[test]
    fn parses_spotify_history_rows() {
        let row = serde_json::json!({
            "ts": "2026-06-01T12:03:30Z",
            "ms_played": 210_000,
            "master_metadata_track_name": "LOUD",
            "master_metadata_album_artist_name": "NMIXX",
            "spotify_track_uri": "spotify:track:abc"
        });
        assert_eq!(
            parse_history_row(&row),
            Some(listen(1780315410 - 210, "NMIXX", "LOUD", "abc", 210_000))
        );

        let short = serde_json::json!({
            "ts": "2026-06-01T12:03:30Z", "ms_played": 29_999,
            "master_metadata_track_name": "LOUD", "master_metadata_album_artist_name": "NMIXX",
            "spotify_track_uri": "spotify:track:abc"
        });
        // 30秒未満も取り込むが、数えない
        assert!(!parse_history_row(&short).unwrap().is_counted());

        let podcast = serde_json::json!({
            "ts": "2026-06-01T12:03:30Z", "ms_played": 600_000,
            "master_metadata_track_name": null, "master_metadata_album_artist_name": null,
            "spotify_track_uri": null, "episode_name": "ep"
        });
        assert_eq!(parse_history_row(&podcast), None);
    }

    #[test]
    fn parses_lastfm_recent_tracks() {
        let json = serde_json::json!({"recenttracks": {
            "track": [
                {"artist": {"#text": "ILLIT"}, "name": "Magnetic", "@attr": {"nowplaying": "true"}},
                {"artist": {"#text": "NMIXX"}, "name": "LOUD", "date": {"uts": "1780315200", "#text": "1 Jun 2026, 12:00"}}
            ],
            "@attr": {"user": "u", "page": "1", "perPage": "200", "totalPages": "3", "total": "401"}
        }});
        let (plays, pages) = parse_recent_tracks(&json).unwrap();
        assert_eq!(plays, vec![scrobble(1780315200, "NMIXX", "LOUD")]);
        assert_eq!(pages, 3);

        // 1件だけのときはオブジェクト
        let single = serde_json::json!({"recenttracks": {
            "track": {"artist": {"#text": "NMIXX"}, "name": "LOUD", "date": {"uts": "1780315200"}},
            "@attr": {"totalPages": "1"}
        }});
        assert_eq!(parse_recent_tracks(&single).unwrap().0.len(), 1);

        let err = serde_json::json!({"error": 10, "message": "Invalid API key"});
        assert!(parse_recent_tracks(&err).unwrap_err().to_string().contains("Invalid API key"));
    }
}
