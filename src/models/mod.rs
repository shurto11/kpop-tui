use serde::{Deserialize, Serialize};

/// クレジットデータ（Geniusからスクレイピング、Writer/Roleごとに1行）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditData {
    pub id: Option<i64>,
    pub artist: String,
    pub label: Option<String>,
    pub date: Option<String>,
    pub album: Option<String>,
    pub track: String,
    pub role: Option<String>,
    pub name: Option<String>,
    pub count: Option<i64>,
    pub created_at: Option<String>,
    pub is_aoty: bool,
    pub is_soty: bool,
}

impl CreditData {
    pub fn new(artist: String, track: String) -> Self {
        Self {
            id: None,
            artist,
            label: None,
            date: None,
            album: None,
            track,
            role: None,
            name: None,
            count: None,
            created_at: None,
            is_aoty: false,
            is_soty: false,
        }
    }
}

/// トラックデータ（曲ごとに1行）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackData {
    pub id: Option<i64>,
    pub artist: String,
    pub label: Option<String>,
    pub date: Option<String>,
    pub album: Option<String>,
    pub track: String,
    pub duration: Option<i64>,
    pub bpm: Option<String>,
    pub spotify: Option<String>,
    pub is_title: bool,
    pub is_prerelease: bool,
    pub is_aoty: bool,
    pub is_soty: bool,
    pub genres: Option<Vec<String>>,
}

impl TrackData {
    pub fn new(track: String, artist: String) -> Self {
        Self {
            id: None,
            artist,
            label: None,
            date: None,
            album: None,
            track,
            duration: None,
            bpm: None,  // String: 数値 or "MIXX"
            spotify: None,
            is_title: false,
            is_prerelease: false,
            is_aoty: false,
            is_soty: false,
            genres: None,
        }
    }
}

/// genres文字列 → Vec
pub fn parse_genres(s: &str) -> Vec<String> {
    s.split(',').map(|g| g.trim().to_string()).filter(|g| !g.is_empty()).collect()
}

/// Vec → DB保存用文字列
pub fn genres_to_string(genres: &[String]) -> String {
    genres.join(",")
}

/// Vec → 表示用 "#EDM #House"
pub fn genres_display(genres: &[String]) -> String {
    genres.iter().map(|g| format!("#{}", g)).collect::<Vec<_>>().join(" ")
}

/// アーティストデータ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistData {
    pub id: Option<i64>,
    pub artist: String,
    pub label: Option<String>,
    pub memo: Option<String>,
    pub sort_order: Option<i64>,
}

impl ArtistData {
    pub fn new(artist: String) -> Self {
        Self {
            id: None,
            artist,
            label: None,
            memo: None,
            sort_order: None,
        }
    }
}

/// ライターデータ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriterData {
    pub id: Option<i64>,
    pub name: String,
    pub real_name: Option<String>,
    pub birth_date: Option<String>,
    pub birth_place: Option<String>,
    pub occupation: Option<String>,
    pub agency: Option<String>,
    pub debut: Option<String>,
    pub memo: Option<String>,
}

impl WriterData {
    pub fn new(name: String) -> Self {
        Self {
            id: None,
            name,
            real_name: None,
            birth_date: None,
            birth_place: None,
            occupation: None,
            agency: None,
            debut: None,
            memo: None,
        }
    }
}

/// クレジット情報（スクレイピング用）
#[derive(Debug, Clone)]
pub struct Credit {
    pub role: String,
    pub name: String,
}

/// Geniusからスクレイピングした曲情報
#[derive(Debug, Clone)]
pub struct ScrapedSongInfo {
    pub artist: String,
    pub album: Option<String>,
    pub date: Option<String>,
    pub track: String,
    pub credits: Vec<Credit>,
}

/// 役割の種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Lyricist,
    Composer,
    Arranger,
    Writer,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Lyricist => "lyricist",
            Role::Composer => "composer",
            Role::Arranger => "arranger",
            Role::Writer => "writer",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "lyricist" | "lyrics" | "lyrics by" => Some(Role::Lyricist),
            "composer" | "composed by" | "music by" => Some(Role::Composer),
            "arranger" | "arranged by" | "arrangement" => Some(Role::Arranger),
            "writer" | "written by" => Some(Role::Writer),
            _ => None,
        }
    }
}

/// songbpm.comからのトラック情報
#[derive(Debug, Clone)]
pub struct BpmTrackInfo {
    pub track_name: String,
    pub duration: Option<String>,
    pub bpm: Option<String>,
    pub spotify_url: Option<String>,
}

/// songbpm.comからのアーティスト情報
#[derive(Debug, Clone)]
pub struct BpmArtistInfo {
    pub artist: String,
    pub tracks: Vec<BpmTrackInfo>,
}

/// 設定ファイル
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub genius: GeniusConfig,
    pub database: DatabaseConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeniusConfig {
    pub header_key: String,
    pub info_key: String,
    pub date_key: String,
    /// Credit__Container/Label/Contributor用キー（新構造）
    #[serde(default)]
    pub credit_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub path: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            genius: GeniusConfig {
                header_key: "908aafe9".to_string(),
                info_key: "56e36c75".to_string(),
                date_key: "hFYGNw".to_string(),
                credit_key: "96426b7f".to_string(),
            },
            database: DatabaseConfig {
                path: "kpop.db".to_string(),
            },
        }
    }
}
