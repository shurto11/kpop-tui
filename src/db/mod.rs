use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

use crate::models::{ArtistData, TrackData, CreditData, WriterData};

pub struct Database {
    conn: Connection,
}

impl Database {
    /// データベースを開く（なければ作成）
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path).context("Failed to open database")?;
        let db = Self { conn };
        db.migrate_rename_tables()?;
        db.init_schema()?;
        Ok(db)
    }

    /// スキーマを初期化
    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            -- 曲データ
            CREATE TABLE IF NOT EXISTS credit_data (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                artist TEXT NOT NULL,
                label TEXT,
                date TEXT,
                album TEXT,
                track TEXT NOT NULL,
                role TEXT,
                name TEXT,
                count INTEGER,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );

            -- 曲追加データ
            CREATE TABLE IF NOT EXISTS track_data (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                track TEXT NOT NULL,
                artist TEXT NOT NULL,
                duration INTEGER,
                bpm TEXT,
                spotify TEXT,
                is_title BOOLEAN DEFAULT TRUE,
                is_prerelease BOOLEAN DEFAULT FALSE,
                is_aoty BOOLEAN DEFAULT FALSE,
                is_soty BOOLEAN DEFAULT FALSE,
                UNIQUE(track, artist)
            );

            -- アーティストデータ
            CREATE TABLE IF NOT EXISTS artist_data (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                artist TEXT UNIQUE NOT NULL,
                label TEXT,
                memo TEXT,
                sort_order INTEGER
            );

            -- ライターデータ
            CREATE TABLE IF NOT EXISTS writer_data (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT UNIQUE NOT NULL,
                real_name TEXT,
                birth_date TEXT,
                birth_place TEXT,
                occupation TEXT,
                agency TEXT,
                debut TEXT,
                memo TEXT
            );

            -- ライターエイリアス（同一人物の別名管理）
            CREATE TABLE IF NOT EXISTS writer_aka (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                primary_name TEXT NOT NULL,
                alias_name TEXT NOT NULL,
                UNIQUE(primary_name, alias_name)
            );

            -- インデックス
            CREATE INDEX IF NOT EXISTS idx_song_artist ON credit_data(artist);
            CREATE INDEX IF NOT EXISTS idx_song_track ON credit_data(track);
            CREATE INDEX IF NOT EXISTS idx_song_name ON credit_data(name);
            CREATE INDEX IF NOT EXISTS idx_song_role ON credit_data(role);
            CREATE INDEX IF NOT EXISTS idx_song_date ON credit_data(date);
            "#,
        )?;

        // マイグレーション: is_track → is_title
        self.migrate_is_track_to_is_title()?;

        // マイグレーション: writer_data から instagram, x, spotify を削除
        self.migrate_drop_writer_socials()?;

        // マイグレーション: is_best16 → is_soty
        self.migrate_best16_to_soty()?;

        // マイグレーション: is_aoty カラム追加
        self.migrate_add_aoty()?;

        Ok(())
    }

    /// テーブル名をモデル名に対応させる（song_data→credit_data, song_add_data→track_data）
    fn migrate_rename_tables(&self) -> Result<()> {
        // song_data が存在すれば credit_data にリネーム
        let has_song_data: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='song_data'",
            [],
            |row| row.get(0),
        )?;
        if has_song_data {
            self.conn.execute_batch("ALTER TABLE song_data RENAME TO credit_data;")?;
        }

        // song_add_data が存在すれば track_data にリネーム
        let has_song_add_data: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='song_add_data'",
            [],
            |row| row.get(0),
        )?;
        if has_song_add_data {
            self.conn.execute_batch("ALTER TABLE song_add_data RENAME TO track_data;")?;
        }

        Ok(())
    }

    /// is_track カラムを is_title にリネーム（既存DB対応）
    fn migrate_is_track_to_is_title(&self) -> Result<()> {
        // カラムが存在するか確認
        let has_is_track: bool = self.conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('track_data') WHERE name = 'is_track'",
            [],
            |row| row.get(0),
        ).unwrap_or(0) > 0;

        if has_is_track {
            // is_track を is_title にリネーム
            self.conn.execute(
                "ALTER TABLE track_data RENAME COLUMN is_track TO is_title",
                [],
            )?;
        }
        Ok(())
    }

    /// writer_data から instagram, x, spotify カラムを削除（既存DB対応）
    fn migrate_drop_writer_socials(&self) -> Result<()> {
        let has_instagram: bool = self.conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('writer_data') WHERE name = 'instagram'",
            [],
            |row| row.get(0),
        ).unwrap_or(0) > 0;

        if has_instagram {
            self.conn.execute("ALTER TABLE writer_data DROP COLUMN instagram", [])?;
            self.conn.execute("ALTER TABLE writer_data DROP COLUMN x", [])?;
            self.conn.execute("ALTER TABLE writer_data DROP COLUMN spotify", [])?;
        }
        Ok(())
    }

    /// is_soty カラムを is_soty にリネーム（既存DB対応）
    fn migrate_best16_to_soty(&self) -> Result<()> {
        let has_best16: bool = self.conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('track_data') WHERE name = 'is_best16'",
            [],
            |row| row.get(0),
        ).unwrap_or(0) > 0;

        if has_best16 {
            self.conn.execute(
                "ALTER TABLE track_data RENAME COLUMN is_best16 TO is_soty",
                [],
            )?;
        }
        Ok(())
    }

    /// is_aoty カラムを追加（既存DB対応）
    fn migrate_add_aoty(&self) -> Result<()> {
        let has_aoty: bool = self.conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('track_data') WHERE name = 'is_aoty'",
            [],
            |row| row.get(0),
        ).unwrap_or(0) > 0;

        if !has_aoty {
            self.conn.execute(
                "ALTER TABLE track_data ADD COLUMN is_aoty BOOLEAN DEFAULT FALSE",
                [],
            )?;
        }
        Ok(())
    }

    // ========== CreditData ==========

    /// CSVからcredit_dataを一括インポート
    pub fn import_credits_from_csv(&self, path: &str) -> Result<usize> {
        let mut reader = csv::Reader::from_path(path)?;
        let mut count = 0;

        let tx = self.conn.unchecked_transaction()?;
        for result in reader.records() {
            let record = result?;
            // CSV列: Artist,Num,Lable,Date,Album,Track,Best16,Role,Name,Count,LOSE YOUR SHXT
            let artist = record.get(0).unwrap_or("").trim();
            let label = record.get(2).unwrap_or("").trim();
            let date = record.get(3).unwrap_or("").trim();
            let album = record.get(4).unwrap_or("").trim();
            let track = record.get(5).unwrap_or("").trim();
            let role = record.get(7).unwrap_or("").trim();
            let name = record.get(8).unwrap_or("").trim();
            let count_val: Option<i64> = record.get(9).and_then(|s| s.trim().parse().ok());

            if artist.is_empty() || track.is_empty() { continue; }

            self.conn.execute(
                r#"INSERT INTO credit_data (artist, label, date, album, track, role, name, count)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
                params![
                    artist,
                    if label.is_empty() { None } else { Some(label) },
                    if date.is_empty() { None } else { Some(date) },
                    if album.is_empty() { None } else { Some(album) },
                    track,
                    if role.is_empty() { None } else { Some(role) },
                    if name.is_empty() { None } else { Some(name) },
                    count_val
                ],
            )?;
            count += 1;
        }
        tx.commit()?;
        Ok(count)
    }

    /// CSVからtrack_dataを一括インポート
    pub fn import_tracks_from_csv(&self, path: &str) -> Result<usize> {
        let mut reader = csv::Reader::from_path(path)?;
        let mut count = 0;

        let tx = self.conn.unchecked_transaction()?;
        for result in reader.records() {
            let record = result?;
            // CSV列: Artist,Num,Lable,Date,Album,Track,Duration,BPM,Spotify,Title,Pre-release
            let artist = record.get(0).unwrap_or("").trim();
            let track = record.get(5).unwrap_or("").trim();
            let duration_str = record.get(6).unwrap_or("").trim();
            let bpm_str = record.get(7).unwrap_or("").trim();
            let spotify = record.get(8).unwrap_or("").trim();
            let title = record.get(9).unwrap_or("").trim();
            let prerelease = record.get(10).unwrap_or("").trim();

            if artist.is_empty() || track.is_empty() { continue; }

            // Duration: "2:48" → 168秒
            let duration: Option<i64> = if duration_str.contains(':') {
                let parts: Vec<&str> = duration_str.split(':').collect();
                if parts.len() == 2 {
                    let m: i64 = parts[0].parse().unwrap_or(0);
                    let s: i64 = parts[1].parse().unwrap_or(0);
                    Some(m * 60 + s)
                } else { None }
            } else {
                duration_str.parse().ok()
            };

            // BPM: 値が2つ（括弧あり）→ "MIXX"、1つ→ そのまま
            let bpm: Option<String> = if bpm_str.is_empty() {
                None
            } else if bpm_str.contains('(') {
                Some("MIXX".to_string())
            } else {
                Some(bpm_str.to_string())
            };

            let is_title = title.eq_ignore_ascii_case("TRUE");
            let is_prerelease = prerelease.eq_ignore_ascii_case("TRUE");

            self.conn.execute(
                r#"INSERT OR REPLACE INTO track_data (track, artist, duration, bpm, spotify, is_title, is_prerelease, is_aoty, is_soty)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, FALSE, FALSE)"#,
                params![
                    track,
                    artist,
                    duration,
                    bpm,
                    if spotify.is_empty() { None } else { Some(spotify) },
                    is_title,
                    is_prerelease
                ],
            )?;
            count += 1;
        }
        tx.commit()?;
        Ok(count)
    }

    /// CSVからartist_dataを一括インポート
    pub fn import_artists_from_csv(&self, path: &str) -> Result<usize> {
        let mut reader = csv::Reader::from_path(path)?;
        let mut count = 0;

        let tx = self.conn.unchecked_transaction()?;
        for result in reader.records() {
            let record = result?;
            // CSV列: Artist,Label,Memo,...
            let artist = record.get(0).unwrap_or("").trim();
            let label = record.get(1).unwrap_or("").trim();
            let memo = record.get(2).unwrap_or("").trim();

            if artist.is_empty() { continue; }

            self.conn.execute(
                r#"INSERT OR IGNORE INTO artist_data (artist, label, memo, sort_order)
                   VALUES (?1, ?2, ?3, ?4)"#,
                params![
                    artist,
                    if label.is_empty() { None } else { Some(label) },
                    if memo.is_empty() { None } else { Some(memo) },
                    count as i64
                ],
            )?;
            count += 1;
        }
        tx.commit()?;
        Ok(count)
    }

    /// CSVからwriter_dataを一括インポート
    pub fn import_writers_from_csv(&self, path: &str) -> Result<usize> {
        let mut reader = csv::Reader::from_path(path)?;
        let mut count = 0;

        let tx = self.conn.unchecked_transaction()?;
        for result in reader.records() {
            let record = result?;
            // CSV列: Name,본명,출생일,출생지,직업,소속사,데뷔,MBTI,메모,...
            let name = record.get(0).unwrap_or("").trim();
            let real_name = record.get(1).unwrap_or("").trim();
            let birth_date = record.get(2).unwrap_or("").trim();
            let birth_place = record.get(3).unwrap_or("").trim();
            let occupation = record.get(4).unwrap_or("").trim();
            let agency = record.get(5).unwrap_or("").trim();
            let debut = record.get(6).unwrap_or("").trim();
            // 7: MBTI (skip)
            let memo = record.get(8).unwrap_or("").trim();

            if name.is_empty() { continue; }

            let o = |s: &str| -> Option<String> { if s.is_empty() { None } else { Some(s.to_string()) } };

            self.conn.execute(
                r#"INSERT OR IGNORE INTO writer_data (name, real_name, birth_date, birth_place, occupation, agency, debut, memo)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
                params![
                    name,
                    o(real_name),
                    o(birth_date),
                    o(birth_place),
                    o(occupation),
                    o(agency),
                    o(debut),
                    o(memo)
                ],
            )?;
            count += 1;
        }
        tx.commit()?;
        Ok(count)
    }

    /// 曲データを挿入
    pub fn insert_song(&self, song: &CreditData) -> Result<i64> {
        self.conn.execute(
            r#"INSERT INTO credit_data (artist, date, album, track, role, name, count)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![
                song.artist,
                song.date,
                song.album,
                song.track,
                song.role,
                song.name,
                song.count
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 曲データを取得（入力順、新しい順）
    pub fn get_songs_by_log(&self) -> Result<Vec<CreditData>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.artist, a.label, s.date, s.album, s.track, s.role, s.name, s.count, s.created_at,
                    COALESCE(t.is_aoty, 0), COALESCE(t.is_soty, 0)
             FROM credit_data s
             LEFT JOIN artist_data a ON s.artist = a.artist
             LEFT JOIN track_data t ON s.artist = t.artist AND s.track = t.track
             ORDER BY s.id DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(CreditData {
                id: Some(row.get(0)?),
                artist: row.get(1)?,
                label: row.get(2)?,
                date: row.get(3)?,
                album: row.get(4)?,
                track: row.get(5)?,
                role: row.get(6)?,
                name: row.get(7)?,
                count: row.get(8)?,
                created_at: row.get(9)?,
                is_aoty: row.get(10)?,
                is_soty: row.get(11)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// 曲データを取得（Artist順、日付順）
    pub fn get_songs_sorted(&self) -> Result<Vec<CreditData>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.artist, a.label, s.date, s.album, s.track, s.role, s.name, s.count, s.created_at,
                    COALESCE(t.is_aoty, 0), COALESCE(t.is_soty, 0)
             FROM credit_data s
             LEFT JOIN artist_data a ON s.artist = a.artist
             LEFT JOIN track_data t ON s.artist = t.artist AND s.track = t.track
             ORDER BY COALESCE(a.sort_order, 999999), s.artist ASC, s.date ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(CreditData {
                id: Some(row.get(0)?),
                artist: row.get(1)?,
                label: row.get(2)?,
                date: row.get(3)?,
                album: row.get(4)?,
                track: row.get(5)?,
                role: row.get(6)?,
                name: row.get(7)?,
                count: row.get(8)?,
                created_at: row.get(9)?,
                is_aoty: row.get(10)?,
                is_soty: row.get(11)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Writer名で曲を検索（エイリアス対応）
    pub fn search_songs_by_writer(&self, writer: &str) -> Result<Vec<CreditData>> {
        let names = self.get_all_names_for_writer(writer)?;
        let placeholders = make_placeholders(names.len());
        let sql = format!(
            "SELECT s.id, s.artist, s.label, s.date, s.album, s.track, s.role, s.name, s.count, s.created_at,
                    COALESCE(t.is_aoty, 0), COALESCE(t.is_soty, 0)
             FROM credit_data s
             LEFT JOIN artist_data a ON s.artist = a.artist
             LEFT JOIN track_data t ON s.artist = t.artist AND s.track = t.track
             WHERE s.name IN ({})
             ORDER BY COALESCE(a.sort_order, 999999), s.artist ASC, s.date ASC,
                      s.track ASC,
                      CASE s.role
                          WHEN 'lyricist' THEN 1
                          WHEN 'composer' THEN 2
                          WHEN 'arranger' THEN 3
                          WHEN 'writer' THEN 4
                          ELSE 5
                      END",
            placeholders
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = names.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
            Ok(CreditData {
                id: Some(row.get(0)?),
                artist: row.get(1)?,
                label: row.get(2)?,
                date: row.get(3)?,
                album: row.get(4)?,
                track: row.get(5)?,
                role: row.get(6)?,
                name: row.get(7)?,
                count: row.get(8)?,
                created_at: row.get(9)?,
                is_aoty: row.get(10)?,
                is_soty: row.get(11)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Artist + Trackで曲を検索
    pub fn search_song(&self, artist: &str, track: &str) -> Result<Vec<CreditData>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.artist, s.label, s.date, s.album, s.track, s.role, s.name,
                    (SELECT COUNT(DISTINCT c2.track || c2.artist) FROM credit_data c2
                     WHERE c2.name = s.name AND c2.role = s.role) as count,
                    s.created_at,
                    COALESCE(t.is_aoty, 0), COALESCE(t.is_soty, 0)
             FROM credit_data s
             LEFT JOIN track_data t ON s.artist = t.artist AND s.track = t.track
             WHERE s.artist = ?1 AND s.track = ?2 ORDER BY s.id ASC",
        )?;
        let rows = stmt.query_map([artist, track], |row| {
            Ok(CreditData {
                id: Some(row.get(0)?),
                artist: row.get(1)?,
                label: row.get(2)?,
                date: row.get(3)?,
                album: row.get(4)?,
                track: row.get(5)?,
                role: row.get(6)?,
                name: row.get(7)?,
                count: row.get(8)?,
                created_at: row.get(9)?,
                is_aoty: row.get(10)?,
                is_soty: row.get(11)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// アーティストの曲一覧
    pub fn get_tracks_by_artist(&self, artist: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT track FROM credit_data WHERE artist = ?1 ORDER BY date DESC",
        )?;
        let rows = stmt.query_map([artist], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// 全アーティスト一覧
    pub fn get_all_artists(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT artist FROM credit_data ORDER BY artist",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Durationが未入力のTrackDataがあるアーティスト一覧
    pub fn get_artists_without_add_data(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT c.artist FROM credit_data c
             WHERE NOT EXISTS (
                 SELECT 1 FROM track_data t WHERE t.artist = c.artist AND t.track = c.track
             )
             ORDER BY c.artist",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// 指定アーティストのTrackDataが未登録の曲一覧
    pub fn get_tracks_without_add_data(&self, artist: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT c.track FROM credit_data c
             WHERE c.artist = ?1
               AND NOT EXISTS (
                   SELECT 1 FROM track_data t WHERE t.artist = c.artist AND t.track = c.track
               )
             ORDER BY c.date DESC",
        )?;
        let rows = stmt.query_map([artist], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Writer名の曲数カウントを更新（エイリアス対応）
    pub fn update_writer_count(&self, name: &str) -> Result<()> {
        let names = self.get_all_names_for_writer(name)?;
        let placeholders = make_placeholders(names.len());
        let sql = format!(
            "SELECT COUNT(DISTINCT track || artist) FROM credit_data WHERE name IN ({})",
            placeholders
        );
        let params: Vec<&dyn rusqlite::types::ToSql> = names.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
        let count: i64 = self.conn.query_row(&sql, rusqlite::params_from_iter(&params), |row| row.get(0))?;
        // 全エイリアス名のcountを更新
        for n in &names {
            self.conn.execute(
                "UPDATE credit_data SET count = ?1 WHERE name = ?2",
                params![count, n],
            )?;
        }
        Ok(())
    }

    // ========== TrackData ==========

    /// 曲追加データを挿入または更新
    pub fn upsert_song_add(&self, data: &TrackData) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO track_data (track, artist, duration, bpm, spotify, is_title, is_prerelease, is_aoty, is_soty)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
               ON CONFLICT(track, artist) DO UPDATE SET
                 duration = excluded.duration,
                 bpm = excluded.bpm,
                 spotify = excluded.spotify,
                 is_title = excluded.is_title,
                 is_prerelease = excluded.is_prerelease,
                 is_aoty = excluded.is_aoty,
                 is_soty = excluded.is_soty"#,
            params![
                data.track,
                data.artist,
                data.duration,
                data.bpm,
                data.spotify,
                data.is_title,
                data.is_prerelease,
                data.is_aoty,
                data.is_soty
            ],
        )?;
        Ok(())
    }

    /// 曲追加データを取得
    pub fn get_song_add(&self, artist: &str, track: &str) -> Result<Option<TrackData>> {
        let result = self.conn.query_row(
            "SELECT id, track, artist, duration, bpm, spotify, is_title, is_prerelease, is_aoty, is_soty
             FROM track_data WHERE artist = ?1 AND track = ?2",
            [artist, track],
            |row| {
                Ok(TrackData {
                    id: Some(row.get(0)?),
                    track: row.get(1)?,
                    artist: row.get(2)?,
                    label: None,
                    date: None,
                    album: None,
                    duration: row.get(3)?,
                    bpm: row.get(4)?,
                    spotify: row.get(5)?,
                    is_title: row.get(6)?,
                    is_prerelease: row.get(7)?,
                    is_aoty: row.get(8)?,
                    is_soty: row.get(9)?,
                })
            },
        );
        match result {
            Ok(data) => Ok(Some(data)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Best16の曲一覧
    pub fn get_soty(&self) -> Result<Vec<TrackData>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.artist, a.label, c.date, c.album, s.track,
                    s.duration, s.bpm, s.spotify, s.is_title, s.is_prerelease, COALESCE(s.is_aoty, 0), s.is_soty
             FROM track_data s
             LEFT JOIN artist_data a ON s.artist = a.artist
             LEFT JOIN (SELECT DISTINCT artist, track, date, album FROM credit_data) c
                    ON s.artist = c.artist AND s.track = c.track
             WHERE s.is_soty = TRUE
             ORDER BY c.date DESC, s.artist, s.track",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(TrackData {
                id: Some(row.get(0)?),
                artist: row.get(1)?,
                label: row.get(2)?,
                date: row.get(3)?,
                album: row.get(4)?,
                track: row.get(5)?,
                duration: row.get(6)?,
                bpm: row.get(7)?,
                spotify: row.get(8)?,
                is_title: row.get(9)?,
                is_prerelease: row.get(10)?,
                is_aoty: row.get(11)?,
                is_soty: row.get(12)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// 全TrackData一覧（Artist順）
    pub fn get_all_track_data(&self) -> Result<Vec<TrackData>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, c.artist, a.label, c.date, c.album, c.track,
                    s.duration, s.bpm, s.spotify,
                    COALESCE(s.is_title, 0), COALESCE(s.is_prerelease, 0), COALESCE(s.is_aoty, 0), COALESCE(s.is_soty, 0)
             FROM (SELECT MIN(rowid) as min_rowid, artist, track, date, album FROM credit_data GROUP BY artist, track, date, album) c
             LEFT JOIN track_data s ON c.artist = s.artist AND c.track = s.track
             LEFT JOIN artist_data a ON c.artist = a.artist
             ORDER BY COALESCE(a.sort_order, 999999), c.artist, c.date ASC, c.min_rowid ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(TrackData {
                id: row.get(0)?,
                artist: row.get(1)?,
                label: row.get(2)?,
                date: row.get(3)?,
                album: row.get(4)?,
                track: row.get(5)?,
                duration: row.get(6)?,
                bpm: row.get(7)?,
                spotify: row.get(8)?,
                is_title: row.get(9)?,
                is_prerelease: row.get(10)?,
                is_aoty: row.get(11)?,
                is_soty: row.get(12)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// AOTYフラグを切り替え
    pub fn toggle_aoty(&self, artist: &str, track: &str) -> Result<bool> {
        let current: bool = self.conn.query_row(
            "SELECT is_aoty FROM track_data WHERE artist = ?1 AND track = ?2",
            [artist, track],
            |row| row.get(0),
        ).unwrap_or(false);

        let new_value = !current;
        self.conn.execute(
            r#"INSERT INTO track_data (track, artist, is_aoty)
               VALUES (?1, ?2, ?3)
               ON CONFLICT(track, artist) DO UPDATE SET is_aoty = excluded.is_aoty"#,
            params![track, artist, new_value],
        )?;
        Ok(new_value)
    }

    /// SOTYフラグを切り替え
    pub fn toggle_soty(&self, artist: &str, track: &str) -> Result<bool> {
        let current: bool = self.conn.query_row(
            "SELECT is_soty FROM track_data WHERE artist = ?1 AND track = ?2",
            [artist, track],
            |row| row.get(0),
        ).unwrap_or(false);

        let new_value = !current;
        self.conn.execute(
            r#"INSERT INTO track_data (track, artist, is_soty)
               VALUES (?1, ?2, ?3)
               ON CONFLICT(track, artist) DO UPDATE SET is_soty = excluded.is_soty"#,
            params![track, artist, new_value],
        )?;
        Ok(new_value)
    }

    // ========== ArtistData ==========

    /// アーティストを挿入または更新
    pub fn upsert_artist(&self, data: &ArtistData) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO artist_data (artist, label, memo, sort_order)
               VALUES (?1, ?2, ?3, ?4)
               ON CONFLICT(artist) DO UPDATE SET
                 label = excluded.label,
                 memo = excluded.memo,
                 sort_order = excluded.sort_order"#,
            params![data.artist, data.label, data.memo, data.sort_order],
        )?;
        Ok(())
    }

    /// アーティスト一覧（sort_order順）
    pub fn get_artists_sorted(&self) -> Result<Vec<ArtistData>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, artist, label, memo, sort_order
             FROM artist_data ORDER BY COALESCE(sort_order, 999999), label, artist",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ArtistData {
                id: Some(row.get(0)?),
                artist: row.get(1)?,
                label: row.get(2)?,
                memo: row.get(3)?,
                sort_order: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// アーティストを削除
    pub fn delete_artist(&self, artist: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM artist_data WHERE artist = ?1",
            [artist],
        )?;
        Ok(())
    }

    /// アーティストの順序を一括更新
    pub fn update_artists_order(&self, artists: &[ArtistData]) -> Result<()> {
        for (i, artist) in artists.iter().enumerate() {
            self.conn.execute(
                "UPDATE artist_data SET sort_order = ?1, label = ?2, memo = ?3 WHERE artist = ?4",
                params![i as i64, &artist.label, &artist.memo, &artist.artist],
            )?;
        }
        Ok(())
    }

    /// ArtistDataテーブルからアーティスト名一覧を取得
    pub fn get_artist_data_names(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT artist FROM artist_data ORDER BY COALESCE(sort_order, 999999), artist",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Labelがないアーティスト一覧
    pub fn get_artists_without_label(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT artist FROM artist_data WHERE label IS NULL OR label = '' ORDER BY artist",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// CreditData(credit_data)に出てくる名前一覧
    pub fn get_credit_names(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT name FROM credit_data WHERE name IS NOT NULL AND name != '' ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// アーティストを取得
    pub fn get_artist(&self, artist: &str) -> Result<Option<ArtistData>> {
        let result = self.conn.query_row(
            "SELECT id, artist, label, memo, sort_order FROM artist_data WHERE artist = ?1",
            [artist],
            |row| {
                Ok(ArtistData {
                    id: Some(row.get(0)?),
                    artist: row.get(1)?,
                    label: row.get(2)?,
                    memo: row.get(3)?,
                    sort_order: row.get(4)?,
                })
            },
        );
        match result {
            Ok(data) => Ok(Some(data)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 全Labelを取得（重複なし）
    pub fn get_all_labels(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT label FROM artist_data WHERE label IS NOT NULL ORDER BY label",
        )?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // ========== WriterData ==========

    /// ライターを挿入または更新
    pub fn upsert_writer(&self, data: &WriterData) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO writer_data (name, real_name, birth_date, birth_place, occupation, agency, debut, memo)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
               ON CONFLICT(name) DO UPDATE SET
                 real_name = excluded.real_name,
                 birth_date = excluded.birth_date,
                 birth_place = excluded.birth_place,
                 occupation = excluded.occupation,
                 agency = excluded.agency,
                 debut = excluded.debut,
                 memo = excluded.memo"#,
            params![
                data.name,
                data.real_name,
                data.birth_date,
                data.birth_place,
                data.occupation,
                data.agency,
                data.debut,
                data.memo
            ],
        )?;
        Ok(())
    }

    /// ライター一覧
    pub fn get_writers(&self) -> Result<Vec<WriterData>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, real_name, birth_date, birth_place, occupation, agency, debut, memo
             FROM writer_data ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(WriterData {
                id: Some(row.get(0)?),
                name: row.get(1)?,
                real_name: row.get(2)?,
                birth_date: row.get(3)?,
                birth_place: row.get(4)?,
                occupation: row.get(5)?,
                agency: row.get(6)?,
                debut: row.get(7)?,
                memo: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// ライターを取得
    pub fn get_writer(&self, name: &str) -> Result<Option<WriterData>> {
        let result = self.conn.query_row(
            "SELECT id, name, real_name, birth_date, birth_place, occupation, agency, debut, memo
             FROM writer_data WHERE name = ?1",
            [name],
            |row| {
                Ok(WriterData {
                    id: Some(row.get(0)?),
                    name: row.get(1)?,
                    real_name: row.get(2)?,
                    birth_date: row.get(3)?,
                    birth_place: row.get(4)?,
                    occupation: row.get(5)?,
                    agency: row.get(6)?,
                    debut: row.get(7)?,
                    memo: row.get(8)?,
                })
            },
        );
        match result {
            Ok(data) => Ok(Some(data)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Writer名の補完候補を取得
    pub fn get_writer_suggestions(&self, prefix: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT name FROM credit_data WHERE name LIKE ?1 ORDER BY name LIMIT 10",
        )?;
        let pattern = format!("{}%", prefix);
        let rows = stmt.query_map([pattern], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // ========== Around-The-Day Drops ==========

    /// リリース日の前日・当日・翌日(MM-DD)に該当する他のTitle/Pre曲を全年横断で検索
    pub fn get_around_day_drops(
        &self,
        md1: &str,
        md2: &str,
        md3: &str,
        exclude_artist: &str,
        exclude_track: &str,
        exclude_album: &str,
    ) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT DISTINCT c.date, s.artist, s.track
               FROM track_data s
               INNER JOIN (SELECT DISTINCT artist, track, date, album FROM credit_data) c
                       ON s.artist = c.artist AND s.track = c.track
               WHERE (s.is_title = TRUE OR s.is_prerelease = TRUE)
                 AND substr(c.date, 6) IN (?1, ?2, ?3)
                 AND NOT (s.artist = ?4 AND s.track = ?5)
                 AND (c.album IS NULL OR c.album = '' OR c.album != ?6)
               ORDER BY c.date"#,
        )?;
        let rows = stmt.query_map(params![md1, md2, md3, exclude_artist, exclude_track, exclude_album], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// 指定MM-DDに発売された曲からランダムに1曲取得（全曲対象、Release不問）
    pub fn get_random_track_by_month_day(&self, md: &str) -> Result<Option<(String, String)>> {
        let result = self.conn.query_row(
            "SELECT DISTINCT c.artist, c.track FROM credit_data c
             INNER JOIN track_data t ON t.artist = c.artist AND t.track = c.track
             WHERE substr(c.date, 6) = ?1 AND t.spotify IS NOT NULL AND t.spotify != ''
             ORDER BY RANDOM() LIMIT 1",
            [md],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match result {
            Ok(data) => Ok(Some(data)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// track_dataに最後に追加された曲を取得（Today's Dropsが無い日のフォールバック用）
    pub fn get_newest_track(&self) -> Result<Option<(String, String)>> {
        let result = self.conn.query_row(
            "SELECT artist, track FROM track_data WHERE spotify IS NOT NULL AND spotify != '' ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match result {
            Ok(data) => Ok(Some(data)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    // ========== 統計 ==========

    /// Writerごとの曲数（Role別、エイリアス対応）
    pub fn get_writer_stats(&self, name: &str) -> Result<Vec<(String, i64)>> {
        let names = self.get_all_names_for_writer(name)?;
        let placeholders = make_placeholders(names.len());
        let sql = format!(
            "SELECT role, COUNT(DISTINCT track || artist) as cnt
             FROM credit_data WHERE name IN ({}) GROUP BY role ORDER BY cnt DESC",
            placeholders
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = names.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Writerの総曲数（ロール重複なし、エイリアス対応）
    pub fn get_writer_total_count(&self, name: &str) -> Result<i64> {
        let names = self.get_all_names_for_writer(name)?;
        let placeholders = make_placeholders(names.len());
        let sql = format!(
            "SELECT COUNT(DISTINCT track || artist) FROM credit_data WHERE name IN ({})",
            placeholders
        );
        let params: Vec<&dyn rusqlite::types::ToSql> = names.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
        self.conn.query_row(&sql, rusqlite::params_from_iter(params), |row| row.get(0)).map_err(Into::into)
    }

    /// WriterのAOTY入り曲数（エイリアス対応）
    pub fn get_writer_aoty_count(&self, name: &str) -> Result<i64> {
        let names = self.get_all_names_for_writer(name)?;
        let placeholders = make_placeholders(names.len());
        let sql = format!(
            r#"SELECT COUNT(DISTINCT s.track || s.artist)
               FROM credit_data s
               JOIN track_data sa ON s.track = sa.track AND s.artist = sa.artist
               WHERE s.name IN ({}) AND sa.is_aoty = TRUE"#,
            placeholders
        );
        let params: Vec<&dyn rusqlite::types::ToSql> = names.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
        self.conn.query_row(&sql, rusqlite::params_from_iter(params), |row| row.get(0)).map_err(Into::into)
    }

    /// WriterのSOTY入り曲数（エイリアス対応）
    pub fn get_writer_soty_count(&self, name: &str) -> Result<i64> {
        let names = self.get_all_names_for_writer(name)?;
        let placeholders = make_placeholders(names.len());
        let sql = format!(
            r#"SELECT COUNT(DISTINCT s.track || s.artist)
               FROM credit_data s
               JOIN track_data sa ON s.track = sa.track AND s.artist = sa.artist
               WHERE s.name IN ({}) AND sa.is_soty = TRUE"#,
            placeholders
        );
        let params: Vec<&dyn rusqlite::types::ToSql> = names.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
        self.conn.query_row(&sql, rusqlite::params_from_iter(params), |row| row.get(0)).map_err(Into::into)
    }

    /// Writerの順位（各ロール・合計・Best16、エイリアス対応）
    pub fn get_writer_ranks(&self, name: &str) -> Result<Vec<(String, i64)>> {
        let primary = self.get_primary_name(name)?;
        let mut ranks = Vec::new();
        // 各ロール別の順位（CTE内でLEFT JOIN writer_akaしてalias解決）
        for role in &["lyricist", "composer", "arranger", "writer"] {
            let rank: i64 = self.conn.query_row(
                "WITH counts AS (
                    SELECT COALESCE(wa.primary_name, c.name) as resolved_name,
                           COUNT(DISTINCT c.track || c.artist) as cnt
                    FROM credit_data c
                    LEFT JOIN writer_aka wa ON c.name = wa.alias_name
                    WHERE c.role = ?1
                    GROUP BY resolved_name
                )
                SELECT COUNT(*) + 1 FROM counts
                WHERE cnt > COALESCE((SELECT cnt FROM counts WHERE resolved_name = ?2), 0)",
                rusqlite::params![role, primary],
                |row| row.get(0),
            )?;
            ranks.push((role.to_string(), rank));
        }
        // 合計の順位
        let total_rank: i64 = self.conn.query_row(
            "WITH counts AS (
                SELECT COALESCE(wa.primary_name, c.name) as resolved_name,
                       COUNT(DISTINCT c.track || c.artist) as cnt
                FROM credit_data c
                LEFT JOIN writer_aka wa ON c.name = wa.alias_name
                GROUP BY resolved_name
            )
            SELECT COUNT(*) + 1 FROM counts
            WHERE cnt > COALESCE((SELECT cnt FROM counts WHERE resolved_name = ?1), 0)",
            [&primary],
            |row| row.get(0),
        )?;
        ranks.push(("total".to_string(), total_rank));
        // SOTYの順位
        let soty_rank: i64 = self.conn.query_row(
            "WITH counts AS (
                SELECT COALESCE(wa.primary_name, s.name) as resolved_name,
                       COUNT(DISTINCT s.track || s.artist) as cnt
                FROM credit_data s
                JOIN track_data sa ON s.track = sa.track AND s.artist = sa.artist
                LEFT JOIN writer_aka wa ON s.name = wa.alias_name
                WHERE sa.is_soty = TRUE
                GROUP BY resolved_name
            )
            SELECT COUNT(*) + 1 FROM counts
            WHERE cnt > COALESCE((SELECT cnt FROM counts WHERE resolved_name = ?1), 0)",
            [&primary],
            |row| row.get(0),
        )?;
        ranks.push(("SOTY".to_string(), soty_rank));
        // AOTYの順位
        let aoty_rank: i64 = self.conn.query_row(
            "WITH counts AS (
                SELECT COALESCE(wa.primary_name, s.name) as resolved_name,
                       COUNT(DISTINCT s.track || s.artist) as cnt
                FROM credit_data s
                JOIN track_data sa ON s.track = sa.track AND s.artist = sa.artist
                LEFT JOIN writer_aka wa ON s.name = wa.alias_name
                WHERE sa.is_aoty = TRUE
                GROUP BY resolved_name
            )
            SELECT COUNT(*) + 1 FROM counts
            WHERE cnt > COALESCE((SELECT cnt FROM counts WHERE resolved_name = ?1), 0)",
            [&primary],
            |row| row.get(0),
        )?;
        ranks.push(("AOTY".to_string(), aoty_rank));
        Ok(ranks)
    }

    /// Writerの年別曲数
    pub fn get_writer_yearly_stats(&self, name: &str) -> Result<Vec<(String, i64)>> {
        let names = self.get_all_names_for_writer(name)?;
        let placeholders = make_placeholders(names.len());
        let sql = format!(
            "SELECT substr(date, 1, 4) as year, COUNT(DISTINCT track || artist) as cnt
             FROM credit_data WHERE name IN ({}) AND date IS NOT NULL
             GROUP BY year ORDER BY year DESC",
            placeholders
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = names.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // ========== WriterAka ==========

    /// 名前からprimary nameを解決（エイリアスならprimary_nameを返す、なければそのまま）
    pub fn get_primary_name(&self, name: &str) -> Result<String> {
        let result = self.conn.query_row(
            "SELECT primary_name FROM writer_aka WHERE alias_name = ?1",
            [name],
            |row| row.get(0),
        );
        match result {
            Ok(primary) => Ok(primary),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                // primary_nameとして登録されているかチェック
                let exists: bool = self.conn.query_row(
                    "SELECT COUNT(*) > 0 FROM writer_aka WHERE primary_name = ?1",
                    [name],
                    |row| row.get(0),
                )?;
                if exists {
                    Ok(name.to_string())
                } else {
                    Ok(name.to_string())
                }
            }
            Err(e) => Err(e.into()),
        }
    }

    /// 名前→primary解決→全aliases(primary自身含む)を収集
    pub fn get_all_names_for_writer(&self, name: &str) -> Result<Vec<String>> {
        let primary = self.get_primary_name(name)?;
        let mut names = vec![primary.clone()];
        let mut stmt = self.conn.prepare(
            "SELECT alias_name FROM writer_aka WHERE primary_name = ?1",
        )?;
        let rows = stmt.query_map([&primary], |row| row.get::<_, String>(0))?;
        for row in rows {
            let alias = row?;
            if alias != primary {
                names.push(alias);
            }
        }
        // primaryがaliasと同じ場合の重複を除去
        names.dedup();
        Ok(names)
    }

    /// エイリアスを追加
    pub fn add_writer_aka(&self, primary_name: &str, alias_name: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO writer_aka (primary_name, alias_name) VALUES (?1, ?2)",
            params![primary_name, alias_name],
        )?;
        Ok(())
    }

    /// エイリアスを削除
    pub fn delete_writer_aka(&self, primary_name: &str, alias_name: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM writer_aka WHERE primary_name = ?1 AND alias_name = ?2",
            params![primary_name, alias_name],
        )?;
        Ok(())
    }

    /// primary_nameに対するエイリアス一覧を取得
    pub fn get_aliases_for_writer(&self, primary_name: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT alias_name FROM writer_aka WHERE primary_name = ?1 ORDER BY alias_name",
        )?;
        let rows = stmt.query_map([primary_name], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// 2つの名前が同一人物（Aka）かどうか判定
    pub fn is_aka_pair(&self, name_a: &str, name_b: &str) -> Result<bool> {
        let primary_a = self.get_primary_name(name_a)?;
        let primary_b = self.get_primary_name(name_b)?;
        Ok(primary_a == primary_b && name_a != name_b)
    }

    /// Akaペアをトグル（ON/OFF切替）
    pub fn toggle_writer_aka(&self, name_a: &str, name_b: &str) -> Result<bool> {
        if self.is_aka_pair(name_a, name_b)? {
            // OFF: 接続を削除
            self.conn.execute(
                "DELETE FROM writer_aka WHERE
                 (primary_name = ?1 AND alias_name = ?2) OR
                 (primary_name = ?2 AND alias_name = ?1)",
                params![name_a, name_b],
            )?;
            // 共有primary経由で接続されている場合
            let primary = self.get_primary_name(name_a).unwrap_or_else(|_| name_a.to_string());
            if primary != name_a && primary != name_b {
                self.conn.execute(
                    "DELETE FROM writer_aka WHERE primary_name = ?1 AND alias_name = ?2",
                    params![&primary, name_b],
                )?;
            }
            Ok(false)
        } else {
            // ON: name_aをprimary、name_bをaliasとして追加
            self.add_writer_aka(name_a, name_b)?;
            Ok(true)
        }
    }

    /// 全テーブルをCSVにエクスポート
    pub fn export_all_csv(&self, dir: &Path) -> Result<(usize, usize, usize, usize, usize)> {
        let c1 = self.export_credits_csv(&dir.join("credit.csv"))?;
        let c2 = self.export_tracks_csv(&dir.join("track.csv"))?;
        let c3 = self.export_artists_csv(&dir.join("artist.csv"))?;
        let c4 = self.export_writers_csv(&dir.join("writer.csv"))?;
        let c5 = self.export_writer_aka_csv(&dir.join("writer_aka.csv"))?;
        Ok((c1, c2, c3, c4, c5))
    }

    fn export_credits_csv(&self, path: &Path) -> Result<usize> {
        let mut wtr = csv::Writer::from_path(path)?;
        wtr.write_record(["Artist", "Num", "Label", "Date", "Album", "Track", "Best16", "Role", "Name", "Count"])?;
        let mut stmt = self.conn.prepare(
            "SELECT artist, label, date, album, track, role, name, count FROM credit_data ORDER BY id"
        )?;
        let mut count = 0;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let artist: String = row.get(0)?;
            let label: Option<String> = row.get(1)?;
            let date: Option<String> = row.get(2)?;
            let album: Option<String> = row.get(3)?;
            let track: String = row.get(4)?;
            let role: Option<String> = row.get(5)?;
            let name: Option<String> = row.get(6)?;
            let cnt: Option<i64> = row.get(7)?;
            wtr.write_record([
                &artist, "", &label.unwrap_or_default(), &date.unwrap_or_default(),
                &album.unwrap_or_default(), &track, "",
                &role.unwrap_or_default(), &name.unwrap_or_default(),
                &cnt.map(|c| c.to_string()).unwrap_or_default(),
            ])?;
            count += 1;
        }
        wtr.flush()?;
        Ok(count)
    }

    fn export_tracks_csv(&self, path: &Path) -> Result<usize> {
        let mut wtr = csv::Writer::from_path(path)?;
        wtr.write_record(["Artist", "Num", "Label", "Date", "Album", "Track", "Duration", "BPM", "Spotify", "Title", "Pre-release", "AOTY", "SOTY"])?;
        let mut stmt = self.conn.prepare(
            "SELECT track, artist, duration, bpm, spotify, is_title, is_prerelease, is_aoty, is_soty FROM track_data ORDER BY id"
        )?;
        let mut count = 0;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let track: String = row.get(0)?;
            let artist: String = row.get(1)?;
            let duration: Option<i64> = row.get(2)?;
            let bpm: Option<String> = row.get(3)?;
            let spotify: Option<String> = row.get(4)?;
            let is_title: bool = row.get(5)?;
            let is_prerelease: bool = row.get(6)?;
            let is_aoty: bool = row.get(7)?;
            let is_soty: bool = row.get(8)?;
            let dur_str = duration.map(|d| format!("{}:{:02}", d / 60, d % 60)).unwrap_or_default();
            let bpm_str = bpm.unwrap_or_default();
            let spotify_str = spotify.unwrap_or_default();
            let title_str = if is_title { "TRUE" } else { "" };
            let pre_str = if is_prerelease { "TRUE" } else { "" };
            let aoty_str = if is_aoty { "TRUE" } else { "" };
            let soty_str = if is_soty { "TRUE" } else { "" };
            wtr.write_record([
                artist.as_str(), "", "", "", "", track.as_str(), dur_str.as_str(),
                bpm_str.as_str(), spotify_str.as_str(),
                title_str, pre_str, aoty_str, soty_str,
            ])?;
            count += 1;
        }
        wtr.flush()?;
        Ok(count)
    }

    fn export_artists_csv(&self, path: &Path) -> Result<usize> {
        let mut wtr = csv::Writer::from_path(path)?;
        wtr.write_record(["Artist", "Label", "Memo"])?;
        let mut stmt = self.conn.prepare(
            "SELECT artist, label, memo FROM artist_data ORDER BY sort_order, id"
        )?;
        let mut count = 0;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let artist: String = row.get(0)?;
            let label: Option<String> = row.get(1)?;
            let memo: Option<String> = row.get(2)?;
            wtr.write_record([&artist, &label.unwrap_or_default(), &memo.unwrap_or_default()])?;
            count += 1;
        }
        wtr.flush()?;
        Ok(count)
    }

    fn export_writers_csv(&self, path: &Path) -> Result<usize> {
        let mut wtr = csv::Writer::from_path(path)?;
        wtr.write_record(["Name", "本名", "出生日", "出生地", "職業", "所属社", "デビュー", "MBTI", "メモ"])?;
        let mut stmt = self.conn.prepare(
            "SELECT name, real_name, birth_date, birth_place, occupation, agency, debut, memo FROM writer_data ORDER BY id"
        )?;
        let mut count = 0;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(0)?;
            let real_name: Option<String> = row.get(1)?;
            let birth_date: Option<String> = row.get(2)?;
            let birth_place: Option<String> = row.get(3)?;
            let occupation: Option<String> = row.get(4)?;
            let agency: Option<String> = row.get(5)?;
            let debut: Option<String> = row.get(6)?;
            let memo: Option<String> = row.get(7)?;
            wtr.write_record([
                &name, &real_name.unwrap_or_default(), &birth_date.unwrap_or_default(),
                &birth_place.unwrap_or_default(), &occupation.unwrap_or_default(),
                &agency.unwrap_or_default(), &debut.unwrap_or_default(),
                "", &memo.unwrap_or_default(),
            ])?;
            count += 1;
        }
        wtr.flush()?;
        Ok(count)
    }

    fn export_writer_aka_csv(&self, path: &Path) -> Result<usize> {
        let mut wtr = csv::Writer::from_path(path)?;
        wtr.write_record(["PrimaryName", "AliasName"])?;
        let mut stmt = self.conn.prepare(
            "SELECT primary_name, alias_name FROM writer_aka ORDER BY id"
        )?;
        let mut count = 0;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let primary: String = row.get(0)?;
            let alias: String = row.get(1)?;
            wtr.write_record([&primary, &alias])?;
            count += 1;
        }
        wtr.flush()?;
        Ok(count)
    }
}

/// IN句用のプレースホルダーを生成 (e.g. "?1, ?2, ?3")
fn make_placeholders(count: usize) -> String {
    (1..=count).map(|i| format!("?{}", i)).collect::<Vec<_>>().join(", ")
}
