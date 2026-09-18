//! Spotifyのお気に入り（Liked Songs）取得。
//!
//! 認証は spotatui に完全に任せる。kpop-tui は spotatui のトークンキャッシュを
//! **読むだけ**で、書き戻しは一切しない（PKCEのrefresh_tokenはローテーションするため、
//! 書き戻すと spotatui 側の認証を壊す恐れがある）。
//! 取得の直前に `spotatui list --liked --limit 1` を一発呼び、期限切れなら
//! spotatui 自身にリフレッシュさせてキャッシュを更新させる。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};

/// お気に入り1曲分
#[derive(Debug, Clone)]
pub struct LikedTrack {
    pub name: String,
    pub artists: Vec<String>,
    /// RFC3339。Spotifyのお気に入り追加日時
    pub added_at: String,
    pub url: Option<String>,
    pub album: Option<String>,
    /// "2026-09-14" または "2026" のような部分日付
    pub release_date: Option<String>,
}

const API_BASE: &str = "https://api.spotify.com/v1/me/tracks";
/// spotatuiのトークン更新を待つ上限（秒）
const NUDGE_TIMEOUT_SECS: u64 = 15;

/// spotatuiにトークンを更新させる。
/// 失敗しても致命ではない（キャッシュがまだ有効な可能性がある）ので Result は返さない。
///
/// spotatui自身にはネットワークタイムアウトがなく、到達不能な環境では永久に待つ。
/// Commandにタイムアウトはないので、自分で期限を測って kill する。
fn nudge_spotatui() {
    let mut child = match std::process::Command::new("spotatui")
        .args(["list", "--liked", "--limit", "1"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        // spotatuiが無い場合はキャッシュをそのまま読みにいく
        Err(_) => return,
    };

    let deadline = std::time::Instant::now() + Duration::from_secs(NUDGE_TIMEOUT_SECS);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return,
        }
    }
}

/// spotatuiのトークンキャッシュのパスを探す。
/// ファイル名は `.spotify_token_cache_<client_id先頭8文字>.json` なので、
/// 決め打ちせずmtimeが最新のものを選ぶ（spotatui自身も同じやり方をしている）。
fn token_cache_path() -> Option<PathBuf> {
    let dir = dirs::config_dir()?.join("spotatui");
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;

    for entry in std::fs::read_dir(&dir).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(".spotify_token_cache") || !name.ends_with(".json") {
            continue;
        }
        let mtime = match entry.metadata().and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if newest.as_ref().map_or(true, |(t, _)| mtime > *t) {
            newest = Some((mtime, entry.path()));
        }
    }

    newest.map(|(_, p)| p)
}

/// キャッシュからアクセストークンを読む。期限切れなら Err。
fn read_access_token() -> Result<String> {
    let path = token_cache_path().ok_or_else(|| {
        anyhow::anyhow!("Spotify token not found. Run spotatui once to authenticate.")
    })?;

    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let json: serde_json::Value =
        serde_json::from_str(&body).context("Failed to parse Spotify token cache")?;

    // 期限確認。自前でリフレッシュはしない（spotatuiの責務）
    if let Some(expires_at) = json["expires_at"].as_str() {
        if let Ok(exp) = chrono::DateTime::parse_from_rfc3339(expires_at) {
            if exp <= chrono::Utc::now() {
                anyhow::bail!("Spotify token expired - open spotatui to refresh");
            }
        }
    }

    json["access_token"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("No access_token in Spotify token cache"))
}

/// JSONの1アイテムを LikedTrack に変換。
/// ローカルファイルやPodcastエピソードは track.id が null なので弾く。
fn parse_item(item: &serde_json::Value) -> Option<LikedTrack> {
    let track = item.get("track")?;
    if track.get("id").map_or(true, |v| v.is_null()) {
        return None;
    }

    let name = track["name"].as_str()?.to_string();
    let artists: Vec<String> = track["artists"]
        .as_array()?
        .iter()
        .filter_map(|a| a["name"].as_str().map(|s| s.to_string()))
        .collect();
    if artists.is_empty() {
        return None;
    }

    Some(LikedTrack {
        name,
        artists,
        added_at: item["added_at"].as_str().unwrap_or("").to_string(),
        url: track["external_urls"]["spotify"]
            .as_str()
            .map(|s| s.to_string()),
        album: track["album"]["name"].as_str().map(|s| s.to_string()),
        release_date: track["album"]["release_date"]
            .as_str()
            .map(|s| s.to_string()),
    })
}

/// お気に入りを全件取得する。Spotify API既定の**新しい順**で返す。
pub fn fetch_liked_tracks(cancel: &AtomicBool) -> Result<Vec<LikedTrack>> {
    // 期限切れならspotatuiに更新させる
    nudge_spotatui();

    let token = read_access_token()?;

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;

    let mut out = Vec::new();
    let mut url = format!("{}?limit=50&offset=0", API_BASE);
    let mut retried_401 = false;
    let mut token = token;

    loop {
        if cancel.load(Ordering::Relaxed) {
            anyhow::bail!("Cancelled");
        }

        let resp = client
            .get(&url)
            .bearer_auth(&token)
            .send()
            .map_err(|e| anyhow::anyhow!("Network error: {}", e))?;

        // トークンが直前に失効した場合、一度だけspotatuiに更新させて再試行
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED && !retried_401 {
            retried_401 = true;
            nudge_spotatui();
            token = read_access_token()?;
            continue;
        }

        if !resp.status().is_success() {
            anyhow::bail!("Spotify API error: {}", resp.status());
        }

        let body = resp.text().context("Failed to read Spotify response")?;
        let json: serde_json::Value =
            serde_json::from_str(&body).context("Failed to parse Spotify response")?;

        if let Some(items) = json["items"].as_array() {
            for item in items {
                if let Some(t) = parse_item(item) {
                    out.push(t);
                }
            }
        }

        // next を辿る（limitの上限は50なのでページングが要る）
        match json["next"].as_str() {
            Some(next) if !next.is_empty() => url = next.to_string(),
            _ => break,
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn dump_liked() {
        let cancel = AtomicBool::new(false);
        let tracks = fetch_liked_tracks(&cancel).expect("fetch");
        println!("total={}", tracks.len());
        for t in tracks.iter().take(5) {
            println!("{} | {} | {} | {:?}", t.added_at, t.name, t.artists.join(", "), t.release_date);
        }
    }
}
