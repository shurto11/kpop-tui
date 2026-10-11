//! 取り込み済みのクレジット名に Genius artist ID を後から付ける（kpop-tui --backfill-genius-ids）。
//!
//! DB の名前は取り込んだ時点の Genius 上の名前なので、その後の改名で今の名前と合わないことがある。
//! 曲ごとに今のクレジットを API で引き、次の順で DB の名前と突き合わせる:
//!   1. 完全一致・表記揺れ（大文字小文字・空白・括弧の並び順）
//!   2. アーティストの別名（alternate_names）に旧名が残っている
//!   3. 旧名のページ URL が今のページへリダイレクトされる（その ID が曲のクレジットにある場合だけ採用）
//! ID は曲ごとに保存するので、途中で止めても次回は続きから進む。

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::time::Duration;

use anyhow::Result;

use crate::db::Database;
use crate::scraper::{self, GeniusArtist};

/// Geniusに連打しない
const INTERVAL: Duration = Duration::from_millis(300);
/// 名前ごとに試す曲数の上限。今のクレジットに見つからない名前のために全曲を回らないようにする
const MAX_TRIES_PER_NAME: usize = 3;

pub fn backfill_genius_ids(db: &Database) -> Result<()> {
    let tracks = db.get_tracks_missing_genius_ids()?;
    println!("{} songs have credits without Genius ID", tracks.len());
    if tracks.is_empty() {
        return Ok(());
    }

    let client = scraper::genius_client()?;
    let mut resolved: HashSet<String> = HashSet::new();
    let mut tries: HashMap<String, usize> = HashMap::new();
    let mut alt_cache: HashMap<u64, Vec<String>> = HashMap::new();
    // ID → Genius 上の今の名前（Aka の primary 選びに使う）
    let mut current_names: HashMap<u64, String> = HashMap::new();
    let mut songs_not_found: Vec<String> = Vec::new();
    let mut consecutive_errors = 0;

    for (i, (artist, track, names)) in tracks.iter().enumerate() {
        let todo: Vec<&String> = names
            .iter()
            .filter(|n| !resolved.contains(*n) && tries.get(*n).copied().unwrap_or(0) < MAX_TRIES_PER_NAME)
            .collect();
        if todo.is_empty() {
            continue;
        }

        print!("[{}/{}] {} - {} ... ", i + 1, tracks.len(), artist, track);
        let _ = std::io::stdout().flush();

        let artists = match song_artists(&client, artist, track) {
            Ok(Some(a)) => {
                consecutive_errors = 0;
                a
            }
            Ok(None) => {
                consecutive_errors = 0;
                println!("song not found");
                songs_not_found.push(format!("{} - {}", artist, track));
                continue;
            }
            Err(e) => {
                println!("error: {}", e);
                consecutive_errors += 1;
                // ネットワークが落ちている／弾かれているときに全件叩き続けない
                if consecutive_errors >= 3 {
                    anyhow::bail!("Stopped after 3 consecutive errors. Run again later to resume.");
                }
                std::thread::sleep(Duration::from_secs(5));
                continue;
            }
        };
        for a in &artists {
            current_names.insert(a.id, a.name.clone());
        }

        let mut matched: Vec<(String, u64)> = Vec::new();
        for name in &todo {
            match match_name(&client, name, &artists, &mut alt_cache) {
                Some(id) => {
                    matched.push(((*name).clone(), id));
                    resolved.insert((*name).clone());
                }
                None => *tries.entry((*name).clone()).or_default() += 1,
            }
        }
        db.set_writer_genius_ids(&matched)?;
        println!("{}/{} matched", matched.len(), todo.len());
    }

    let linked = db.sync_aka_for_genius_ids(&db.get_all_genius_ids()?, &current_names)?;
    let unresolved: Vec<&String> = tries.keys().filter(|n| !resolved.contains(*n)).collect();

    println!();
    println!("Matched {} names", resolved.len());
    if !linked.is_empty() {
        println!("Linked {} names as Aka:", linked.len());
        for (primary, alias) in &linked {
            println!("  {} ← {}", primary, alias);
        }
    }
    if !unresolved.is_empty() {
        let mut unresolved = unresolved;
        unresolved.sort();
        println!("Not matched ({}): use WriterAka for these", unresolved.len());
        for name in unresolved {
            println!("  {}", name);
        }
    }
    if !songs_not_found.is_empty() {
        println!("Songs not found on Genius ({}):", songs_not_found.len());
        for s in &songs_not_found {
            println!("  {}", s);
        }
    }
    Ok(())
}

/// 曲を探して、クレジットされているアーティストを ID 付きで返す
fn song_artists(
    client: &reqwest::blocking::Client,
    artist: &str,
    track: &str,
) -> Result<Option<Vec<GeniusArtist>>> {
    std::thread::sleep(INTERVAL);
    let Some(song_id) = scraper::find_song_id(client, artist, track)? else {
        return Ok(None);
    };
    std::thread::sleep(INTERVAL);
    Ok(Some(scraper::fetch_song_artists(client, song_id)?))
}

/// DB の名前を曲の今のクレジットと突き合わせて ID を返す
fn match_name(
    client: &reqwest::blocking::Client,
    name: &str,
    artists: &[GeniusArtist],
    alt_cache: &mut HashMap<u64, Vec<String>>,
) -> Option<u64> {
    if let Some(a) = scraper::match_artist(name, artists) {
        return Some(a.id);
    }

    // 改名前の名前が別名に残っていることが多い（"Haewon (NMIXX)" → "HAEWON (해원)"）
    for a in artists.iter().filter(|a| a.is_writer) {
        let alts = alt_cache.entry(a.id).or_insert_with(|| {
            std::thread::sleep(INTERVAL);
            scraper::fetch_alternate_names(client, a.id).unwrap_or_default()
        });
        if alts.iter().any(|alt| scraper::same_credit_name(alt, name)) {
            return Some(a.id);
        }
    }

    // 別名に残っていない改名も、旧URLのリダイレクトで辿れる。
    // ただし同名の別人のページに当たることがあるので、曲のクレジットにある ID だけ採る
    std::thread::sleep(INTERVAL);
    let id = scraper::resolve_artist_by_old_name(client, name).ok().flatten()?;
    artists.iter().any(|a| a.id == id).then_some(id)
}
