use kpop_tui::{backfill, db, models, plays, tui};

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use db::Database;
use models::Config;
use plays::PlayDb;
use tui::{app::App, input::{finish_auto_add_bpm, finish_auto_add_scan, handle_key, process_auto_add_msg, process_bpm_result, process_scrape_result, tick_bpm_pending, quiz_auto_play}, ui::draw};

fn main() -> Result<()> {
    // 設定ファイルのパス
    let config_path = get_config_path();
    let config = load_or_create_config(&config_path)?;

    // データベースのパス
    let db_path = get_data_dir().join(&config.database.path);
    let db = Database::open(&db_path)?;
    let play_db = PlayDb::open(get_data_dir().join("plays.db"))?;

    // CSVインポートモード
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "--import-credit" {
        let count = db.import_credits_from_csv(&args[2])?;
        println!("Imported {} credit records.", count);
        return Ok(());
    }
    if args.len() >= 3 && args[1] == "--import-track" {
        let count = db.import_tracks_from_csv(&args[2])?;
        println!("Imported {} track records.", count);
        return Ok(());
    }
    if args.len() >= 3 && args[1] == "--import-artist" {
        let count = db.import_artists_from_csv(&args[2])?;
        println!("Imported {} artist records.", count);
        return Ok(());
    }
    if args.len() >= 3 && args[1] == "--import-writer" {
        let count = db.import_writers_from_csv(&args[2])?;
        println!("Imported {} writer records.", count);
        return Ok(());
    }

    // 取り込み済みのクレジット名に Genius artist ID を付け、同じ ID の名前を Aka に紐付ける
    if args.len() >= 2 && args[1] == "--backfill-genius-ids" {
        backfill::backfill_genius_ids(&db)?;
        return Ok(());
    }

    // Spotify のデータダウンロード（拡張ストリーミング再生履歴）を再生回数に取り込む
    if args.len() >= 3 && args[1] == "--import-spotify-history" {
        let history = plays::read_spotify_history(Path::new(&args[2]))?;
        let inserted = play_db.insert_spotify(&history.plays)?;
        println!(
            "Read {} rows from {} files: {} song plays ({} of 30s+ are counted), {} new.",
            history.rows,
            history.files,
            history.plays.len(),
            history.plays.iter().filter(|p| p.is_counted()).count(),
            inserted
        );
        print_play_summary(&db, &play_db)?;
        return Ok(());
    }

    // Last.fm の scrobble を取り込む（TUI 起動中は自動で同期される）
    if args.len() >= 2 && args[1] == "--sync-lastfm" {
        let lastfm = config
            .lastfm()
            .ok_or_else(|| anyhow::anyhow!("[lastfm] user and api_key are not set in config.toml"))?;
        let fetched = plays::fetch_lastfm(lastfm, play_db.last_lastfm_at()?)?;
        let inserted = play_db.insert_lastfm(&fetched)?;
        println!("Fetched {} scrobbles, {} new.", fetched.len(), inserted);
        print_play_summary(&db, &play_db)?;
        return Ok(());
    }

    // CSVエクスポートモード
    if args.len() >= 2 && args[1] == "--export" {
        let dir = if args.len() >= 3 { &args[2] } else { "backup" };
        let export_dir = get_data_dir().join(dir);
        std::fs::create_dir_all(&export_dir)?;
        let counts = db.export_all_csv(&export_dir)?;
        println!("Exported to {}:", export_dir.display());
        println!("  credit_data:  {} records", counts.0);
        println!("  track_data:   {} records", counts.1);
        println!("  artist_data:  {} records", counts.2);
        println!("  writer_data:  {} records", counts.3);
        println!("  writer_aka:   {} records", counts.4);
        return Ok(());
    }

    // ターミナル初期化
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // アプリケーション起動
    let mut app = App::new(db, play_db, config);
    let result = run_app(&mut terminal, &mut app);

    // ターミナル復元
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Error: {}", e);
    }

    // 終了時に自動バックアップ
    let backup_dir = get_data_dir().join("backup");
    if let Err(e) = std::fs::create_dir_all(&backup_dir)
        .map_err(anyhow::Error::from)
        .and_then(|_| app.db.export_all_csv(&backup_dir))
    {
        eprintln!("Backup failed: {}", e);
    }

    // 終了時にSQLダンプをkpop-tui-dataリポジトリへpush（変更があるときのみ）
    let backup_script = get_data_dir().join("scripts").join("backup-data.sh");
    if backup_script.exists() {
        match std::process::Command::new(&backup_script)
            .env("KPOP_DB", &db_path)
            .status()
        {
            Ok(s) if s.success() => {}
            Ok(s) => eprintln!("Data backup failed: {}", s),
            Err(e) => eprintln!("Data backup failed: {}", e),
        }
    }

    Ok(())
}

/// アプリケーションメインループ
fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    while app.running {
        terminal.draw(|f| draw(f, app))?;

        // tick更新（スピナーアニメーション用）
        app.tick = app.tick.wrapping_add(1);

        // スクレイピング結果の受信チェック
        if let Some(rx) = app.scrape_receiver.take() {
            match rx.try_recv() {
                Ok(result) => {
                    app.loading = false;
                    process_scrape_result(app, result);
                    continue;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // まだ受信していない → receiverを戻す
                    app.scrape_receiver = Some(rx);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // スレッドが異常終了
                    app.loading = false;
                    app.show_error("Scraping thread disconnected");
                }
            }
        }

        // BPMスクレイピング結果の受信チェック
        if let Some(rx) = app.bpm_receiver.take() {
            match rx.try_recv() {
                Ok(result) => {
                    app.loading = false;
                    process_bpm_result(app, result);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    app.bpm_receiver = Some(rx);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.loading = false;
                    app.show_error("BPM search thread disconnected");
                }
            }
        }

        // アルバムアート受信チェック
        if let Some(rx) = app.album_art_receiver.take() {
            match rx.try_recv() {
                Ok(Ok((url, lines))) => {
                    app.album_art_cache.insert(url, lines.clone());
                    app.album_art_current = Some(lines);
                }
                Ok(Err(_)) => {
                    // サイレント失敗 → アートなしレイアウト
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    app.album_art_receiver = Some(rx);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // サイレント失敗
                }
            }
        }

        // BPMマッチを1件ずつ追加（件数を徐々に増やす）
        tick_bpm_pending(app);

        // Spotify再生結果の受信チェック
        if let Some(rx) = app.spotify_receiver.take() {
            match rx.try_recv() {
                Ok(Ok(msg)) => {
                    app.spotify_playing = true;
                    app.show_message(&msg);
                }
                Ok(Err(msg)) => {
                    app.spotify_playing = false;
                    app.show_error(&msg);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    app.spotify_receiver = Some(rx);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.spotify_playing = false;
                    app.show_error("Spotify process disconnected");
                }
            }
        }

        // AutoAdd受信チェック。
        // 1ループ1件だとevent::poll(100ms)に律速されて進捗が実際より数秒遅れて見えるので、
        // 溜まっている分をまとめて吸い出す。
        if app.auto_add_rx.is_some() {
            let mut disconnected = false;
            loop {
                let msg = match app.auto_add_rx.as_ref().unwrap().try_recv() {
                    Ok(m) => m,
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                };
                process_auto_add_msg(app, msg);
            }
            // ワーカーが送り終えてtxをdropした＝正常終了。エラーではない
            if disconnected {
                app.auto_add_rx = None;
                finish_auto_add_scan(app);
            }
        }

        // AutoAdd BPM受信チェック（Geniusチェックとは別のワーカー）
        if app.auto_add_bpm_rx.is_some() {
            let mut disconnected = false;
            loop {
                let msg = match app.auto_add_bpm_rx.as_ref().unwrap().try_recv() {
                    Ok(m) => m,
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                };
                process_auto_add_msg(app, msg);
            }
            if disconnected {
                app.auto_add_bpm_rx = None;
                finish_auto_add_bpm(app);
            }
        }

        // Last.fm の同期（間隔が空いていれば開始し、終わっていれば取り込む）
        app.maybe_start_lastfm_sync();
        if let Some(rx) = app.lastfm_rx.take() {
            match rx.try_recv() {
                Ok(result) => app.finish_lastfm_sync(result),
                Err(std::sync::mpsc::TryRecvError::Empty) => app.lastfm_rx = Some(rx),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    app.finish_lastfm_sync(Err("Last.fm sync thread disconnected".to_string()));
                }
            }
        }

        // イベント待機（100ms タイムアウト）
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                // ローディング中はEscでキャンセルのみ受付
                if app.loading {
                    if key.code == crossterm::event::KeyCode::Esc {
                        app.loading = false;
                        app.scrape_receiver = None;
                        app.bpm_receiver = None;
                        app.spotify_receiver = None;
                        app.show_message("Cancelled");
                    }
                } else {
                    let prev_screen = app.screen.clone();
                    handle_key(app, key);
                    // Quiz開始時のSpotify自動再生（QuizResultからの遷移は除外：既にpre-play済み）
                    if matches!(app.screen, tui::Screen::Quiz)
                        && !matches!(prev_screen, tui::Screen::Quiz | tui::Screen::QuizResult)
                        && !app.quiz_questions.is_empty()
                    {
                        quiz_auto_play(app);
                    }
                }
            }
        }
    }

    Ok(())
}

/// 取り込み後に、再生回数が付いた曲数と上位を出す
fn print_play_summary(db: &Database, play_db: &PlayDb) -> Result<()> {
    let counts = play_db.play_counts(&db.get_all_track_data()?)?;
    let total: i64 = counts.values().sum();
    println!("{} plays on {} tracks in TrackData.", total, counts.len());

    let mut top: Vec<_> = counts.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for ((artist, track), n) in top.iter().take(10) {
        println!("  {:>5}  {} - {}", n, artist, track);
    }
    Ok(())
}

/// データディレクトリを取得
fn get_data_dir() -> PathBuf {
    // ~/ssd/tui/kpop-tui/
    dirs::home_dir()
        .map(|h| h.join("ssd").join("tui").join("kpop-tui"))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 設定ファイルのパスを取得
fn get_config_path() -> PathBuf {
    get_data_dir().join("config.toml")
}

/// 設定ファイルを読み込むか作成
fn load_or_create_config(path: &PathBuf) -> Result<Config> {
    if path.exists() {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    } else {
        // デフォルト設定を作成
        let config = Config::default();
        let content = toml::to_string_pretty(&config)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(config)
    }
}
