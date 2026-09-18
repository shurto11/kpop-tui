use anyhow::{Context, Result};
use regex::Regex;
use scraper::{Html, Selector};

use crate::models::{BpmArtistInfo, BpmTrackInfo, Config, Credit, ScrapedSongInfo};

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

/// Geniusから曲情報をスクレイピング
pub fn scrape_genius(url: &str, config: &Config) -> Result<ScrapedSongInfo> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/115.0.0.0 Safari/537.36")
        .build()?;

    let response = client
        .get(url)
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Referer", "https://genius.com/")
        .send()
        .context("Failed to fetch page")?;

    let html = response.text().context("Failed to read response")?;
    parse_genius_html(&html, config)
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

/// songbpm.comのURL生成
pub fn make_songbpm_url(artist: &str) -> String {
    let slug = clean_text(artist);
    format!("https://songbpm.com/@{}", slug)
}

/// songbpm.comからBPM情報をスクレイピング
pub fn scrape_songbpm(url: &str) -> Result<BpmArtistInfo> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/115.0.0.0 Safari/537.36")
        .build()?;

    let mut all_tracks: Vec<BpmTrackInfo> = Vec::new();
    let mut current_url = url.to_string();
    let artist_name = extract_artist_from_url(url);

    // ページネーションを辿って全トラックを取得
    loop {
        let response = client
            .get(&current_url)
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .context("Failed to fetch songbpm page")?;

        let html = response.text().context("Failed to read songbpm response")?;
        let (tracks, next_url) = parse_songbpm_html(&html, &artist_name)?;
        all_tracks.extend(tracks);

        match next_url {
            Some(next) => {
                current_url = if next.starts_with("http") {
                    next
                } else {
                    format!("https://songbpm.com{}", next)
                };
            }
            None => break,
        }
    }

    Ok(BpmArtistInfo {
        artist: artist_name,
        tracks: all_tracks,
    })
}

/// URLからアーティスト名を抽出
fn extract_artist_from_url(url: &str) -> String {
    url.split("/@")
        .nth(1)
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .to_string()
}

/// songbpm.comのHTMLをパースしてトラック情報を抽出
fn parse_songbpm_html(html: &str, _artist: &str) -> Result<(Vec<BpmTrackInfo>, Option<String>)> {
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

    // 次ページリンク
    let next_selector = Selector::parse("a[href*='after=']")
        .map_err(|e| anyhow::anyhow!("Invalid next selector: {:?}", e))?;

    let mut tracks: Vec<BpmTrackInfo> = Vec::new();

    for card in document.select(&card_selector) {
        // トラック名を取得（2番目のpタグ = 曲名）
        let ps: Vec<_> = card.select(&p_selector).collect();
        let track_name = if ps.len() >= 2 {
            ps[1].text().collect::<String>().trim().to_string()
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
            track_name,
            duration,
            bpm,
            spotify_url,
        });
    }

    // 次ページURLを取得
    let next_url = document
        .select(&next_selector)
        .next()
        .and_then(|el| el.value().attr("href"))
        .map(|s| s.to_string());

    Ok((tracks, next_url))
}

/// アーティスト名を正規化（括弧内韓国語の除去・残存韓国語文字の除去）
/// 例: "Billlie (빌리)" → "Billlie", "(G)I-DLE" → "(G)I-DLE"（韓国語のない括弧は保持）
pub fn normalize_artist_name(name: &str) -> String {
    // 括弧内に韓国語が1文字以上含まれるグループを除去
    let re = Regex::new(r"\s*[\(\[（【][^\)\]）】]*[\u{AC00}-\u{D7A3}][^\)\]）】]*[\)\]）】]").unwrap();
    let result = re.replace_all(name, "");
    // 残った韓国語文字（ハングル音節ブロック）を除去
    let result: String = result
        .chars()
        .filter(|c| !('\u{AC00}'..='\u{D7A3}').contains(c))
        .collect();
    result.trim().to_string()
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
