use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use regex::Regex;
use scraper::{Html, Selector};

use crate::models::{BpmTrackInfo, Config, Credit, ScrapedSongInfo};

/// GeniusのURL生成
pub fn make_url(artist: &str, track: &str) -> String {
    let artist_clean = clean_text(artist);
    let track_clean = clean_text(track);
    format!("https://genius.com/{}-{}-lyrics", artist_clean, track_clean)
}

/// テキストをURLに適した形式にクリーンアップ
fn clean_text(text: &str) -> String {
    let mut result = text.to_lowercase();

    // 韓国語削除 (U+AC00-U+D7A3)
    result = result
        .chars()
        .filter(|c| !('\u{AC00}'..='\u{D7A3}').contains(c))
        .collect();

    // 括弧を空白に変換
    result = result.replace(['(', ')'], " ");

    // 特殊文字を削除
    result = result.replace(['.', ',', ':', '\''], "");

    // & を and に変換
    result = result.replace('&', "and");

    // æ を削除
    result = result.replace('æ', "");

    // 空白を - に変換
    result = result.split_whitespace().collect::<Vec<_>>().join("-");

    // 連続する - を1つにまとめる
    let re = Regex::new(r"-+").unwrap();
    result = re.replace_all(&result, "-").to_string();

    // 先頭と末尾の - を削除
    result.trim_matches('-').to_string()
}

/// Genius用のHTTPクライアント。タイムアウトなしだとハングした1本がワーカーを永久に止める
fn genius_client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/115.0.0.0 Safari/537.36")
        .timeout(Duration::from_secs(10))
        .build()?)
}

/// 1ページ取得する。ステータスコードとボディを返す。
fn genius_get(client: &reqwest::blocking::Client, url: &str) -> Result<(reqwest::StatusCode, String)> {
    let response = client
        .get(url)
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Referer", "https://genius.com/")
        .send()
        .context("Failed to fetch page")?;

    let status = response.status();
    let html = response.text().context("Failed to read response")?;
    Ok((status, html))
}

/// Geniusから曲情報をスクレイピング
pub fn scrape_genius(url: &str, config: &Config) -> Result<ScrapedSongInfo> {
    let client = genius_client()?;
    let (status, html) = genius_get(&client, url)?;

    // 存在しない曲でもGeniusは404と一緒に長いHTMLを返す。
    // ステータスを見ないと、404ページのタイトルから偽のartist/trackを拾ってしまう。
    if status == reqwest::StatusCode::NOT_FOUND {
        anyhow::bail!("Page not found (404): {}", url);
    }
    if !status.is_success() {
        anyhow::bail!("HTTP {} from Genius", status);
    }

    parse_genius_html(&html, config)
}

/// Geniusページ存在確認の結果
#[derive(Debug)]
pub enum GeniusCheck {
    /// 2xxかつパース成功。
    /// failed_url は、第1候補が404で第2候補以降が当たった場合の「外れたURL」
    Found {
        url: String,
        info: Box<ScrapedSongInfo>,
        failed_url: Option<String>,
    },
    /// 全候補が404
    NotFound { tried: Vec<String> },
    /// 通信失敗・タイムアウト・その他ステータス。ページ不存在とは区別する
    NetworkError(String),
}

/// 曲名から `(feat. ...)` / `(with ...)` を除去
fn strip_feat(track: &str) -> String {
    let re = Regex::new(r"(?i)\s*[\(\[]\s*(feat\.?|ft\.?|with)\s[^\)\]]*[\)\]]").unwrap();
    re.replace_all(track, "").trim().to_string()
}

/// 曲名の ` - ...` 以降を除去（"SIGN - Japanese Ver." → "SIGN"）
fn strip_dash_suffix(track: &str) -> String {
    match track.find(" - ") {
        Some(pos) => track[..pos].trim().to_string(),
        None => track.trim().to_string(),
    }
}

/// アーティスト名から2人目以降を落とす（"A & B" / "A, B" → "A"）
fn primary_artist(artist: &str) -> String {
    let re = Regex::new(r"\s*(,|&|feat\.?|ft\.?|with)\s+").unwrap();
    match re.find(artist) {
        Some(m) => artist[..m.start()].trim().to_string(),
        None => artist.trim().to_string(),
    }
}

/// 括弧を「空白に変換」ではなく「そのまま削除」する。
/// clean_text は `(` を空白にするので "ALL(H)OURS" → "all-h-ours" になるが、
/// Geniusの実際のスラッグは "allhours"。両方試す必要がある
fn strip_parens(s: &str) -> String {
    s.replace(['(', ')', '[', ']', '（', '）'], "")
}

/// 末尾の括弧グループを丸ごと落とす（"Touch (Y2K Unit)" → "Touch"）。
/// Geniusはバージョン表記を入れないことが多い
fn strip_trailing_parens(s: &str) -> String {
    let re = Regex::new(r"\s*[\(\[（][^\)\]）]*[\)\]）]\s*$").unwrap();
    re.replace(s, "").trim().to_string()
}

/// アクセント付きラテン文字をASCIIに落とす（"México" → "Mexico"）。
/// clean_textはASCII以外を素通しするので、そのままではスラッグが合わない
fn fold_accents(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' => 'A',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'É' | 'È' | 'Ê' | 'Ë' => 'E',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'Í' | 'Ì' | 'Î' | 'Ï' => 'I',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' => 'O',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'Ú' | 'Ù' | 'Û' | 'Ü' => 'U',
            'ñ' => 'n',
            'Ñ' => 'N',
            'ç' => 'c',
            'Ç' => 'C',
            other => other,
        })
        .collect()
}

/// 試すGenius URLの候補を順に組み立てる。
/// Spotifyの曲名には `(feat. X)` や `- Japanese Ver.` が付いていてそのままでは404になり、
/// 括弧入りのアーティスト名もスラッグの作り方が2通りあるため、段階的に削って数本試す。
/// 重複は除き、最大4本まで。
pub fn genius_url_candidates(artist: &str, track: &str) -> Vec<String> {
    let no_feat = strip_feat(track);
    let no_dash = strip_dash_suffix(&no_feat);
    let main_artist = primary_artist(artist);

    // 末尾の括弧を落とした形（"Touch (Y2K Unit)" → "Touch"）
    let no_paren_suffix = strip_trailing_parens(&no_dash);

    let variants: Vec<(String, String)> = vec![
        // (1) そのまま
        (artist.to_string(), track.to_string()),
        // (2) 括弧を削除したアーティスト名（"ALL(H)OURS" → "allhours"）
        (strip_parens(artist), track.to_string()),
        // (3) feat. 除去
        (artist.to_string(), no_feat.clone()),
        // (4) " - ..." 以降を除去（"SIGN - Japanese Ver." → "SIGN"）
        (artist.to_string(), no_dash.clone()),
        // (5) 末尾の括弧グループを除去（"Touch (Y2K Unit)" → "Touch"）
        (artist.to_string(), no_paren_suffix.clone()),
        // (6) 括弧削除 ＋ 曲名も整理
        (strip_parens(artist), no_paren_suffix.clone()),
        // (7) 主アーティストのみ
        (main_artist, no_paren_suffix),
    ];

    let mut urls = Vec::new();
    for (a, t) in variants {
        if a.trim().is_empty() || t.trim().is_empty() {
            continue;
        }
        // アクセントを残した形と落とした形の両方を試す（"México" → "mexico"）
        for (a, t) in [
            (a.clone(), t.clone()),
            (fold_accents(&a), fold_accents(&t)),
        ] {
            let url = make_url(&a, &t);
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
        if urls.len() >= 5 {
            break;
        }
    }
    urls.truncate(5);
    urls
}

/// 候補URLを順に試して、最初に見つかったページの情報を返す
pub fn check_genius_candidates(
    artist: &str,
    track: &str,
    config: &Config,
    cancel: &AtomicBool,
) -> GeniusCheck {
    let client = match genius_client() {
        Ok(c) => c,
        Err(e) => return GeniusCheck::NetworkError(e.to_string()),
    };

    let candidates = genius_url_candidates(artist, track);
    let mut tried: Vec<String> = Vec::new();

    for url in &candidates {
        if cancel.load(Ordering::Relaxed) {
            return GeniusCheck::NetworkError("Cancelled".to_string());
        }

        let (status, html) = match genius_get(&client, url) {
            Ok(v) => v,
            Err(e) => return GeniusCheck::NetworkError(format!("{}", e)),
        };

        if status == reqwest::StatusCode::NOT_FOUND {
            tried.push(url.clone());
            continue;
        }

        // 429はレート制限。Retry-Afterを待って1回だけやり直す
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            std::thread::sleep(Duration::from_secs(5));
            match genius_get(&client, url) {
                Ok((s2, h2)) if s2.is_success() => {
                    return finish_found(url, &h2, config, &tried);
                }
                Ok((s2, _)) => {
                    return GeniusCheck::NetworkError(format!("HTTP {} from Genius", s2));
                }
                Err(e) => return GeniusCheck::NetworkError(format!("{}", e)),
            }
        }

        // 403などCloudflare由来のものは「存在しない」ではなくネットワーク側の問題として扱う
        if !status.is_success() {
            return GeniusCheck::NetworkError(format!("HTTP {} from Genius", status));
        }

        return finish_found(url, &html, config, &tried);
    }

    GeniusCheck::NotFound { tried }
}

/// 2xxが返ったページをパースして GeniusCheck に詰める
fn finish_found(url: &str, html: &str, config: &Config, tried: &[String]) -> GeniusCheck {
    match parse_genius_html(html, config) {
        Ok(info) => GeniusCheck::Found {
            url: url.to_string(),
            info: Box::new(info),
            // 第1候補が外れていたら、その外れたURLを残す
            failed_url: tried.first().cloned(),
        },
        Err(_) => {
            // 2xxだがパースできない＝実質見つからなかった扱い
            let mut all = tried.to_vec();
            all.push(url.to_string());
            GeniusCheck::NotFound { tried: all }
        }
    }
}

/// HTMLをパースして曲情報を抽出
fn parse_genius_html(html: &str, config: &Config) -> Result<ScrapedSongInfo> {
    let document = Html::parse_document(html);

    // アーティスト名
    let artist_selector = Selector::parse(&format!(
        "div.SongHeader-desktop__CreditList-sc-{}-16 a.StyledLink-sc-15c685a-0",
        config.genius.header_key
    ))
    .map_err(|e| anyhow::anyhow!("Invalid artist selector: {:?}", e))?;

    let mut artist = document
        .select(&artist_selector)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .unwrap_or_default();

    // 曲名
    let title_selector = Selector::parse(&format!(
        "h1.SongHeader-desktop__Title-sc-{}-9 span.SongHeader-desktop__HiddenMask-sc-{}-13",
        config.genius.header_key, config.genius.header_key
    ))
    .map_err(|e| anyhow::anyhow!("Invalid title selector: {:?}", e))?;

    let mut track = document
        .select(&title_selector)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .unwrap_or_default();

    // リリース日（設定キーで試み、失敗したら全spanからdateパターンを検索）
    let date = {
        let configured = if !config.genius.date_key.is_empty() {
            // クラス名にdate_keyが含まれるspanを検索
            let span_sel = Selector::parse("span").ok();
            span_sel.and_then(|sel| {
                document.select(&sel).find_map(|el| {
                    let has_key = el.value().classes().any(|c| c == config.genius.date_key);
                    if has_key {
                        let raw = el.text().collect::<String>().trim().to_string();
                        parse_date(&raw)
                    } else {
                        None
                    }
                })
            })
        } else {
            None
        };

        // フォールバック: 全spanから日付パターンで検索
        configured.or_else(|| {
            let span_sel = Selector::parse("span").ok()?;
            document.select(&span_sel).find_map(|el| {
                let text = el.text().collect::<String>().trim().to_string();
                parse_date(&text)
            })
        })
    };

    // アルバム名
    let album_selector = Selector::parse("a[href=\"#primary-album\"]")
        .map_err(|e| anyhow::anyhow!("Invalid album selector: {:?}", e))?;

    let album = document
        .select(&album_selector)
        .next()
        .map(|el| {
            el.text()
                .filter(|t| !t.trim().is_empty())
                .collect::<String>()
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty());

    // クレジット情報（新構造: Credit__Container → 旧構造: SongInfo__Credit にフォールバック）
    let link_selector = Selector::parse("a")
        .map_err(|e| anyhow::anyhow!("Invalid link selector: {:?}", e))?;

    let mut credits: Vec<Credit> = Vec::new();
    let mut has_composer = false;

    // 新構造: Credit__Container / Credit__Label / Credit__Contributor
    if !config.genius.credit_key.is_empty() {
        let new_credit_selector = Selector::parse(&format!(
            ".Credit__Container-sc-{}-0",
            config.genius.credit_key
        ))
        .map_err(|e| anyhow::anyhow!("Invalid new credit selector: {:?}", e))?;

        let new_label_selector = Selector::parse(&format!(
            ".Credit__Label-sc-{}-1",
            config.genius.credit_key
        ))
        .map_err(|e| anyhow::anyhow!("Invalid new label selector: {:?}", e))?;

        for credit_div in document.select(&new_credit_selector) {
            if let Some(label_el) = credit_div.select(&new_label_selector).next() {
                let role_raw = label_el.text().collect::<String>().trim().to_lowercase();
                let role = if role_raw.ends_with('s') && role_raw != "lyrics" {
                    role_raw.trim_end_matches('s').to_string()
                } else {
                    role_raw
                };
                let normalized_role = normalize_role(&role);
                if let Some(normalized) = normalized_role {
                    if normalized == "composer" {
                        has_composer = true;
                    }
                    for link in credit_div.select(&link_selector) {
                        let name = link.text().collect::<String>().trim().to_string();
                        if !name.is_empty() {
                            credits.push(Credit {
                                role: normalized.to_string(),
                                name,
                            });
                        }
                    }
                }
            }
        }
    }

    // 旧構造にフォールバック: SongInfo__Credit / SongInfo__Label
    if credits.is_empty() {
        let old_credit_selector = Selector::parse(&format!(
            ".SongInfo__Credit-sc-{}-3",
            config.genius.info_key
        ))
        .map_err(|e| anyhow::anyhow!("Invalid old credit selector: {:?}", e))?;

        let old_label_selector = Selector::parse(&format!(
            ".SongInfo__Label-sc-{}-4",
            config.genius.info_key
        ))
        .map_err(|e| anyhow::anyhow!("Invalid old label selector: {:?}", e))?;

        for credit_div in document.select(&old_credit_selector) {
            if let Some(label_el) = credit_div.select(&old_label_selector).next() {
                let role_raw = label_el.text().collect::<String>().trim().to_lowercase();
                let role = if role_raw.ends_with('s') && role_raw != "lyrics" {
                    role_raw.trim_end_matches('s').to_string()
                } else {
                    role_raw
                };
                let normalized_role = normalize_role(&role);
                if let Some(normalized) = normalized_role {
                    if normalized == "composer" {
                        has_composer = true;
                    }
                    for link in credit_div.select(&link_selector) {
                        let name = link.text().collect::<String>().trim().to_string();
                        if !name.is_empty() {
                            credits.push(Credit {
                                role: normalized.to_string(),
                                name,
                            });
                        }
                    }
                }
            }
        }
    }

    // composerがある場合はwriterを除外
    if has_composer {
        credits.retain(|c| c.role != "writer");
    }

    // Lyricist → Composer → Arranger → Writer の順にソート
    credits.sort_by_key(|c| role_order(&c.role));

    if artist.is_empty() || track.is_empty() {
        // Fallback: try meta[property="og:title"] content or <title> tag to derive artist and track.
        let title_meta_sel = Selector::parse("meta[property=\"og:title\"]").ok();
        if let Some(sel) = title_meta_sel {
            if let Some(content) = document.select(&sel).next().and_then(|e| e.value().attr("content")) {
                let mut s = content.trim().to_string();
                // Remove trailing " | Genius" or " Lyrics"
                if let Some(pos) = s.rfind(" | Genius") { s.truncate(pos); }
                if s.to_lowercase().ends_with(" lyrics") {
                    s.truncate(s.len().saturating_sub(7));
                    s = s.trim().to_string();
                }
                // Try splitting by common separators
                let seps = [" – ", " — ", " - ", "—", "–", " - "];
                let mut parts: Vec<&str> = Vec::new();
                for sep in &seps {
                    if s.contains(sep) {
                        parts = s.splitn(2, sep).collect();
                        break;
                    }
                }
                if parts.is_empty() {
                    parts = s.splitn(2, " - ").collect();
                }
                if artist.is_empty() { artist = parts.get(0).map(|p| p.trim().to_string()).unwrap_or_default(); }
                if track.is_empty() { track = parts.get(1).map(|p| p.trim().to_string()).unwrap_or_default(); }
            }
        }

        // Another fallback: <title> tag
        if (artist.is_empty() || track.is_empty()) {
            if let Some(title_el) = document.select(&Selector::parse("title").unwrap()).next() {
                let mut s = title_el.text().collect::<String>().trim().to_string();
                if let Some(pos) = s.rfind(" | Genius") { s.truncate(pos); }
                if s.to_lowercase().ends_with(" lyrics") { s.truncate(s.len().saturating_sub(7)); s = s.trim().to_string(); }
                let seps = [" – ", " — ", " - ", "—", "–", " - "];
                let mut parts: Vec<&str> = Vec::new();
                for sep in &seps {
                    if s.contains(sep) {
                        parts = s.splitn(2, sep).collect();
                        break;
                    }
                }
                if parts.is_empty() { parts = s.splitn(2, " - ").collect(); }
                if artist.is_empty() { artist = parts.get(0).map(|p| p.trim().to_string()).unwrap_or_default(); }
                if track.is_empty() { track = parts.get(1).map(|p| p.trim().to_string()).unwrap_or_default(); }
            }
        }

        if artist.is_empty() || track.is_empty() {
            anyhow::bail!("Failed to extract artist or track name from page");
        }
    }

    Ok(ScrapedSongInfo {
        artist,
        album,
        date,
        track,
        credits,
    })
}

/// 役割を正規化
fn normalize_role(role: &str) -> Option<&'static str> {
    match role {
        "lyricist" | "lyric" | "lyrics" | "lyrics by" => Some("lyricist"),
        "composer" | "composed by" | "music by" | "music" => Some("composer"),
        "arranger" | "arranged by" | "arrangement" => Some("arranger"),
        "writer" | "written by" | "writing" => Some("writer"),
        _ => None,
    }
}

/// 役割のソート順序（Lyricist → Composer → Arranger → Writer）
fn role_order(role: &str) -> u8 {
    match role {
        "lyricist" => 0,
        "composer" => 1,
        "arranger" => 2,
        "writer" => 3,
        _ => 99,
    }
}

/// 日付をパース（"Jan 1, 2024" -> "2024-01-01"）
fn parse_date(raw: &str) -> Option<String> {
    let clean = raw.replace('.', "");

    // chrono で パース
    if let Ok(dt) = chrono::NaiveDate::parse_from_str(&clean, "%b %d, %Y") {
        return Some(dt.format("%Y-%m-%d").to_string());
    }

    // 別のフォーマットを試す
    if let Ok(dt) = chrono::NaiveDate::parse_from_str(&clean, "%B %d, %Y") {
        return Some(dt.format("%Y-%m-%d").to_string());
    }

    None
}

/// songbpm.comから「アーティスト 曲名」のBPM候補を取得。
/// アーティストページを先頭から辿ると10件ずつのページングで
/// 曲数の多いアーティストだと数十秒かかるため、曲単位で引く。
pub fn search_songbpm(artist: &str, track: &str) -> Result<Vec<BpmTrackInfo>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/115.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    // アーティストページは曲名順に並び、?after=<曲名slug> で目的の曲の位置へ直接飛べる。
    // ただしslugの付け方が一定でなく（"fromis9" / "g-i-dle"）、同じアーティストが
    // 複数slugに分かれていることもある（"all-h-ours" と "allhours"）ので両方引く。
    // 検索もあいまいで上位5件しか返らず、"ALL(H)OURS DANG DANG" のように
    // アーティスト名を含めると目的の曲が落ちることがあるので、曲名のみの検索も合わせる
    let mut requests: Vec<SongbpmRequest> = Vec::new();
    let track_slug = songbpm_slug(track);
    if !track_slug.is_empty() {
        let compact = songbpm_slug(artist).replace('-', "");
        for artist_slug in [songbpm_slug(artist), compact] {
            if artist_slug.is_empty() {
                continue;
            }
            let url = format!("https://songbpm.com/@{}?after={}", artist_slug, track_slug);
            if !requests.iter().any(|r| matches!(r, SongbpmRequest::ArtistPage(u) if *u == url)) {
                requests.push(SongbpmRequest::ArtistPage(url));
            }
        }
    }
    requests.push(SongbpmRequest::Search(format!("{} {}", artist, track)));
    requests.push(SongbpmRequest::Search(track.to_string()));
    let found = search_songbpm_queries(&client, &requests, artist, track)?;
    if !found.is_empty() {
        return Ok(sort_by_release_date(&client, found));
    }

    // それでも落ちる曲がある（"ENHYPEN Bloody Paradise" は外れ、
    // "ENHYPEN Paradise" なら当たる）ので、アーティスト＋曲名の単語1つずつで再検索
    let mut words: Vec<String> = Vec::new();
    for w in normalize_track_name(track).split_whitespace() {
        if w.len() >= 2 && !words.iter().any(|x| x == w) {
            words.push(w.to_string());
        }
    }
    if words.len() < 2 {
        return Ok(found);
    }
    let requests: Vec<SongbpmRequest> = words
        .iter()
        .take(4)
        .map(|w| SongbpmRequest::Search(format!("{} {}", artist, w)))
        .collect();
    let found = search_songbpm_queries(&client, &requests, artist, track)?;
    Ok(sort_by_release_date(&client, found))
}

/// 候補をSpotifyのリリース日の古い順に並べる。
/// 同じ曲がコンピレーション等で別IDとして複数登録されているので、
/// 最も古いもの（オリジナル）を先頭にする。日付が取れないものは末尾
fn sort_by_release_date(
    client: &reqwest::blocking::Client,
    tracks: Vec<BpmTrackInfo>,
) -> Vec<BpmTrackInfo> {
    if tracks.len() < 2 {
        return tracks;
    }
    let dates: Vec<Option<String>> = std::thread::scope(|s| {
        let handles: Vec<_> = tracks
            .iter()
            .map(|t| {
                s.spawn(move || {
                    t.spotify_url
                        .as_deref()
                        .and_then(|url| fetch_spotify_release_date(client, url))
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().ok().flatten()).collect()
    });

    let mut paired: Vec<(Option<String>, BpmTrackInfo)> = dates.into_iter().zip(tracks).collect();
    // ISO 8601なので文字列比較で日付順になる。同日は元の順を保つ
    paired.sort_by(|(a, _), (b, _)| match (a, b) {
        (Some(a), Some(b)) => a.cmp(b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    paired.into_iter().map(|(_, t)| t).collect()
}

/// Spotifyの埋め込みページ（認証不要）からトラックのリリース日を取得
fn fetch_spotify_release_date(client: &reqwest::blocking::Client, track_url: &str) -> Option<String> {
    let id = track_url.split("/track/").nth(1)?.split(['?', '/']).next()?;
    let html = client
        .get(format!("https://open.spotify.com/embed/track/{}", id))
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .text()
        .ok()?;
    let doc = Html::parse_document(&html);
    let selector = Selector::parse("script#__NEXT_DATA__").ok()?;
    let json_text = doc.select(&selector).next()?.text().collect::<String>();
    let json: serde_json::Value = serde_json::from_str(&json_text).ok()?;
    json["props"]["pageProps"]["state"]["data"]["entity"]["releaseDate"]["isoString"]
        .as_str()
        .map(|s| s.to_string())
}

/// songbpm.comへの1リクエスト
enum SongbpmRequest {
    /// 検索クエリ
    Search(String),
    /// アーティストページのURL
    ArtistPage(String),
}

/// songbpm.comのslug（小文字、英数字以外は "-" 区切り）
fn songbpm_slug(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// 複数リクエストを並列で投げ、アーティスト・曲名で絞った結果を返す。
/// 全リクエストが失敗した場合のみエラー
fn search_songbpm_queries(
    client: &reqwest::blocking::Client,
    requests: &[SongbpmRequest],
    artist: &str,
    track: &str,
) -> Result<Vec<BpmTrackInfo>> {
    let responses: Vec<Result<Vec<BpmTrackInfo>>> = std::thread::scope(|s| {
        let handles: Vec<_> = requests
            .iter()
            .map(|r| {
                s.spawn(move || match r {
                    SongbpmRequest::Search(q) => fetch_songbpm_search(client, q),
                    SongbpmRequest::ArtistPage(url) => fetch_songbpm_page(client, url),
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| Err(anyhow::anyhow!("songbpm search panicked"))))
            .collect()
    });

    let mut results: Vec<BpmTrackInfo> = Vec::new();
    let mut last_err = None;
    let mut any_ok = false;
    for r in responses {
        match r {
            Ok(v) => {
                any_ok = true;
                results.extend(v);
            }
            Err(e) => last_err = Some(e),
        }
    }
    if !any_ok {
        if let Some(e) = last_err {
            return Err(e);
        }
    }

    // 検索結果には他アーティストの曲も混ざるので絞る
    let target = normalize_artist_for_bpm(artist);
    let mut seen = std::collections::HashSet::new();
    let by_artist: Vec<BpmTrackInfo> = results
        .into_iter()
        .filter(|t| {
            // 正規化で空になる名前（ハングルのみ等）は判定できないので通す
            if target.is_empty() {
                return true;
            }
            let a = normalize_artist_for_bpm(&t.artist);
            if !a.is_empty() && (a.contains(&target) || target.contains(&a)) {
                return true;
            }
            // グループ名義のソロ曲は曲名側に名前が入る
            // 例: IVE "Force (ANYUJIN Solo)"
            normalize_artist_for_bpm(&t.track_name).contains(&target)
        })
        // 複数の検索で同じ曲が重複するので除く。
        // Spotify IDが違えば別の版（コンピレーション等）なので残す
        .filter(|t| {
            seen.insert((
                t.track_name.clone(),
                t.bpm.clone(),
                t.duration.clone(),
                t.spotify_url.clone(),
            ))
        })
        .collect();

    Ok(find_all_tracks_in_bpm_data(&by_artist, track)
        .into_iter()
        .cloned()
        .collect())
}

/// songbpm.comのアーティストページを1ページ取得して結果カードをパース。
/// slugが存在しなければ404でエラーになる
fn fetch_songbpm_page(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<BpmTrackInfo>> {
    let html = client
        .get(url)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .context("Failed to fetch songbpm page")?
        .error_for_status()
        .context("songbpm page not found")?
        .text()
        .context("Failed to read songbpm response")?;

    parse_songbpm_html(&html)
}

/// songbpm.comに検索クエリを投げて結果カードをパース
fn fetch_songbpm_search(client: &reqwest::blocking::Client, query: &str) -> Result<Vec<BpmTrackInfo>> {
    // SvelteKitのCSRFチェックがあり、Originがないと403になる。
    // 成功すると302で /searches/<id> の結果ページへ飛ぶ
    let html = client
        .post("https://songbpm.com/searches")
        .header("Origin", "https://songbpm.com")
        .header("Accept-Language", "en-US,en;q=0.9")
        .form(&[("query", query)])
        .send()
        .context("Failed to search songbpm")?
        .error_for_status()
        .context("songbpm search failed")?
        .text()
        .context("Failed to read songbpm response")?;

    parse_songbpm_html(&html)
}

/// アーティスト名の照合用正規化（"(G)I-DLE" と "i-dle" を近づけるため空白も除去）
fn normalize_artist_for_bpm(name: &str) -> String {
    normalize_track_name(name).replace(' ', "")
}

/// songbpm.comのHTMLをパースしてトラック情報を抽出
fn parse_songbpm_html(html: &str) -> Result<Vec<BpmTrackInfo>> {
    let document = Html::parse_document(html);

    // トラックカードのコンテナ: div.bg-card
    let card_selector = Selector::parse("div.bg-card")
        .map_err(|e| anyhow::anyhow!("Invalid card selector: {:?}", e))?;

    // テキスト要素
    let span_selector = Selector::parse("span")
        .map_err(|e| anyhow::anyhow!("Invalid span selector: {:?}", e))?;

    let p_selector = Selector::parse("p")
        .map_err(|e| anyhow::anyhow!("Invalid p selector: {:?}", e))?;

    // Spotifyリンク
    let spotify_selector = Selector::parse("a[href*='open.spotify.com']")
        .map_err(|e| anyhow::anyhow!("Invalid spotify selector: {:?}", e))?;

    let mut tracks: Vec<BpmTrackInfo> = Vec::new();

    for card in document.select(&card_selector) {
        // 1番目のpタグ = アーティスト名、2番目 = 曲名
        let ps: Vec<_> = card.select(&p_selector).collect();
        let (artist, track_name) = if ps.len() >= 2 {
            (
                ps[0].text().collect::<String>().trim().to_string(),
                ps[1].text().collect::<String>().trim().to_string(),
            )
        } else {
            continue;
        };

        if track_name.is_empty() {
            continue;
        }

        // Key, Duration, BPM を span から抽出
        let mut duration: Option<String> = None;
        let mut bpm: Option<String> = None;

        let spans: Vec<_> = card.select(&span_selector).collect();
        for (i, span) in spans.iter().enumerate() {
            let text = span.text().collect::<String>().trim().to_string();
            match text.as_str() {
                "Duration" => {
                    if let Some(next_span) = spans.get(i + 1) {
                        let val = next_span.text().collect::<String>().trim().to_string();
                        if !val.is_empty() {
                            duration = Some(val);
                        }
                    }
                }
                "BPM" => {
                    if let Some(next_span) = spans.get(i + 1) {
                        let val = next_span.text().collect::<String>().trim().to_string();
                        if !val.is_empty() { bpm = Some(val); }
                    }
                }
                _ => {}
            }
        }

        // Spotify URL を取得
        let spotify_url = card
            .select(&spotify_selector)
            .next()
            .and_then(|el| el.value().attr("href"))
            .map(|s| s.to_string());

        tracks.push(BpmTrackInfo {
            artist,
            track_name,
            duration,
            bpm,
            spotify_url,
        });
    }

    Ok(tracks)
}

/// アーティスト名を正規化（括弧内韓国語の除去・残存韓国語文字の除去）
/// 例: "Billlie (빌리)" → "Billlie", "(G)I-DLE" → "(G)I-DLE"（韓国語のない括弧は保持）
pub fn normalize_artist_name(name: &str) -> String {
    let name = strip_feat_and_stray_parens(name);
    // 括弧内に韓国語が1文字以上含まれるグループを除去
    let re = Regex::new(r"\s*[\(\[（【][^\)\]）】]*[\u{AC00}-\u{D7A3}][^\)\]）】]*[\)\]）】]").unwrap();
    let result = re.replace_all(&name, "");
    // 残った韓国語文字（ハングル音節ブロック）を除去
    let result: String = result
        .chars()
        .filter(|c| !('\u{AC00}'..='\u{D7A3}').contains(c))
        .collect();
    result.trim().to_string()
}

/// ゼロ幅文字、"(Ft. ...)" グループ（入れ子の括弧を含む）、対応する開き括弧のない ")" を除去
/// 例: "MASHIRO (Ft. BOBBY (바비))" → "MASHIRO", "MASHIRO)" → "MASHIRO"
fn strip_feat_and_stray_parens(name: &str) -> String {
    // ゼロ幅文字（"\u{200B}pH-1" など）は見た目が同じで別名扱いになるため除去
    let chars: Vec<char> = name
        .chars()
        .filter(|c| !matches!(c, '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}'))
        .collect();
    let mut out = String::new();
    let mut depth = 0usize;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '(' {
            let rest: String = chars[i + 1..].iter().collect::<String>().to_lowercase();
            if rest.starts_with("ft.") || rest.starts_with("feat.") {
                // 対応する ")" まで読み飛ばす
                let mut d = 0usize;
                while i < chars.len() {
                    match chars[i] {
                        '(' => d += 1,
                        ')' => {
                            d -= 1;
                            if d == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
                continue;
            }
            depth += 1;
        } else if c == ')' {
            if depth == 0 {
                i += 1;
                continue;
            }
            depth -= 1;
        }
        out.push(c);
        i += 1;
    }
    out.trim().to_string()
}

/// トラック名をマッチング用に正規化
pub fn normalize_track_name(name: &str) -> String {
    let mut result = name.to_lowercase();

    // 括弧を削除（中身は残す）
    result = result.replace(['(', ')', '（', '）', '[', ']'], "");

    // ASCII英数字とスペースのみ残す（ハングル・漢字・特殊文字すべて除去）
    result = result
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || c.is_ascii_whitespace())
        .collect();

    // 空白を正規化
    result = result.split_whitespace().collect::<Vec<_>>().join(" ");
    result.trim().to_string()
}

/// BPMデータからトラック名で全マッチを検索
pub fn find_all_tracks_in_bpm_data<'a>(
    tracks: &'a [BpmTrackInfo],
    target: &str,
) -> Vec<&'a BpmTrackInfo> {
    let normalized_target = normalize_track_name(target);

    // 完全一致を集める
    let exact: Vec<_> = tracks.iter().filter(|t| {
        normalize_track_name(&t.track_name) == normalized_target
    }).collect();
    if !exact.is_empty() {
        return exact;
    }

    // 部分一致（ターゲットがBPMデータのトラック名に含まれる、またはその逆）
    tracks.iter().filter(|t| {
        let normalized = normalize_track_name(&t.track_name);
        normalized.contains(&normalized_target) || normalized_target.contains(&normalized)
    }).collect()
}

/// ASCIIアートの1ピクセル（文字 + RGB色）
pub type AsciiPixel = (char, u8, u8, u8);
/// ASCIIアート全体（行×列）
pub type AsciiArt = Vec<Vec<AsciiPixel>>;

/// Spotify oEmbed APIからアルバムアートのサムネイルURLだけを取得（ブラウザ版用）
pub fn fetch_album_art_url(spotify_url: &str) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0")
        // ネットワーク断でHTTPハンドラを止めないよう必ず打ち切る
        .timeout(std::time::Duration::from_secs(8))
        .build()?;

    let oembed_url = format!(
        "https://open.spotify.com/oembed?url={}",
        urlencoding_manual(spotify_url)
    );

    let body = client
        .get(&oembed_url)
        .send()
        .context("Failed to fetch oEmbed")?
        .text()
        .context("Failed to read oEmbed response")?;

    let json: serde_json::Value =
        serde_json::from_str(&body).context("Failed to parse oEmbed JSON")?;

    json["thumbnail_url"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("No thumbnail_url in oEmbed response"))
}

/// Spotify oEmbed APIからアルバムアートを取得してカラーASCIIアートに変換
pub fn fetch_album_art_ascii(spotify_url: &str, width: u32, height: u32) -> Result<AsciiArt> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0")
        .build()?;

    // oEmbed API呼び出し
    let oembed_url = format!(
        "https://open.spotify.com/oembed?url={}",
        urlencoding_manual(spotify_url)
    );

    let response = client
        .get(&oembed_url)
        .send()
        .context("Failed to fetch oEmbed")?;

    let body = response.text().context("Failed to read oEmbed response")?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .context("Failed to parse oEmbed JSON")?;

    let thumbnail_url = json["thumbnail_url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No thumbnail_url in oEmbed response"))?;

    // サムネイル画像をダウンロード
    let img_bytes = client
        .get(thumbnail_url)
        .send()
        .context("Failed to fetch thumbnail")?
        .bytes()
        .context("Failed to read thumbnail bytes")?;

    // 画像をデコード＆リサイズ
    let img = image::load_from_memory(&img_bytes)
        .context("Failed to decode image")?
        .resize_exact(width, height, image::imageops::FilterType::Lanczos3)
        .to_rgb8();

    // ASCII文字セット（暗い→明るい、ターミナルdark bg向け反転）
    let ascii_chars: &[u8] = b" .:-=+*#%@";

    let mut lines: AsciiArt = Vec::with_capacity(height as usize);
    for y in 0..height {
        let mut row: Vec<AsciiPixel> = Vec::with_capacity(width as usize);
        for x in 0..width {
            let pixel = img.get_pixel(x, y);
            let r = pixel[0];
            let g = pixel[1];
            let b = pixel[2];
            // ITU-R BT.601 輝度計算
            let luminance = 0.299 * r as f64
                + 0.587 * g as f64
                + 0.114 * b as f64;
            let idx = ((luminance / 255.0) * (ascii_chars.len() - 1) as f64).round() as usize;
            let idx = idx.min(ascii_chars.len() - 1);
            row.push((ascii_chars[idx] as char, r, g, b));
        }
        lines.push(row);
    }

    Ok(lines)
}

/// 最低限のURLエンコード
fn urlencoding_manual(url: &str) -> String {
    let mut result = String::with_capacity(url.len() * 2);
    for c in url.chars() {
        match c {
            ' ' => result.push_str("%20"),
            ':' | '/' | '?' | '=' | '&' | '#' | '.' | '-' | '_' | '~'
            | 'a'..='z' | 'A'..='Z' | '0'..='9' => result.push(c),
            _ => {
                let mut buf = [0u8; 4];
                let encoded = c.encode_utf8(&mut buf);
                for b in encoded.bytes() {
                    result.push_str(&format!("%{:02X}", b));
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_artist_name() {
        assert_eq!(normalize_artist_name("Billlie (빌리)"), "Billlie");
        assert_eq!(normalize_artist_name("Billlie"), "Billlie");
        assert_eq!(normalize_artist_name("IVE 아이브"), "IVE");
        assert_eq!(normalize_artist_name("(G)I-DLE"), "(G)I-DLE");
        assert_eq!(normalize_artist_name("aespa [에스파]"), "aespa");
        assert_eq!(normalize_artist_name("MASHIRO (Ft. BOBBY (바비))"), "MASHIRO");
        assert_eq!(normalize_artist_name("pH-1 (Ft. g0nny (거니))"), "pH-1");
        assert_eq!(normalize_artist_name("MASHIRO)"), "MASHIRO");
        assert_eq!(normalize_artist_name("\u{200B}pH-1 & KEITA"), "pH-1 & KEITA");
        assert_eq!(normalize_artist_name("ALL(H)OURS"), "ALL(H)OURS");
        assert_eq!(normalize_artist_name("KAI (EXO)"), "KAI (EXO)");
    }

    #[test]
    fn test_clean_text() {
        assert_eq!(clean_text("BTS"), "bts");
        assert_eq!(clean_text("New Jeans"), "new-jeans");
        assert_eq!(clean_text("(G)I-DLE"), "g-i-dle");
        assert_eq!(clean_text("IVE 아이브"), "ive");
        assert_eq!(clean_text("Tom & Jerry"), "tom-and-jerry");
    }

    #[test]
    fn test_make_url() {
        let url = make_url("NewJeans", "Hype Boy");
        assert_eq!(url, "https://genius.com/newjeans-hype-boy-lyrics");
    }

    #[test]
    fn test_parse_date() {
        assert_eq!(parse_date("Jan 1, 2024"), Some("2024-01-01".to_string()));
        assert_eq!(parse_date("Dec. 25, 2023"), Some("2023-12-25".to_string()));
    }
}

#[cfg(test)]
mod autoadd_tests {
    use super::*;

    #[test]
    fn candidates_strip_suffixes() {
        let c = genius_url_candidates("izna", "SIGN - Japanese Ver.");
        println!("izna/SIGN: {:?}", c);
        assert!(c.iter().any(|u| u.ends_with("izna-sign-lyrics")));

        let c = genius_url_candidates("LISA", "Rockstar (feat. Foo)");
        println!("LISA: {:?}", c);
        assert!(c.iter().any(|u| u.ends_with("lisa-rockstar-lyrics")));

        let c = genius_url_candidates("A & B", "Song");
        println!("A&B: {:?}", c);
        assert!(c.len() <= 3);
    }

    #[test]
    #[ignore]
    fn live_check() {
        let cfg = Config::default();
        let cancel = AtomicBool::new(false);
        for (a, t) in [
            ("Hearts2Hearts", "Moonride"),
            ("ALL(H)OURS", "DANG DANG"),
            ("izna", "SIGN - Japanese Ver."),
            ("VERIVERY", "Touch (Y2K Unit)"),
            ("CHUNG HA", "México"),
        ] {
            let r = check_genius_candidates(a, t, &cfg, &cancel);
            match &r {
                GeniusCheck::Found { url, info, failed_url } => {
                    println!("OK   {} / {} -> {} (was: {:?}) date={:?}", a, t, url, failed_url, info.date);
                }
                GeniusCheck::NotFound { tried } => println!("MISS {} / {} tried={:?}", a, t, tried),
                GeniusCheck::NetworkError(e) => println!("NET  {} / {} {}", a, t, e),
            }
        }
    }
}
