use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use crate::db::Database;
use crate::models::{ArtistData, BpmArtistInfo, BpmTrackInfo, Config, ScrapedSongInfo, TrackData, CreditData, WriterData};

/// 操作モード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Search,
    Visual,  // ViewArtistData用: 並び替え・削除
}

/// 画面の種類
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    // メインメニュー
    MainMenu,
    // Input
    InputMenu,
    InputAutoAdd,
    InputCreditData,
    InputTrackData,
    InputArtistData,
    InputWriterData,
    InputWriterAka,
    // Search
    SearchMenu,
    SearchWriter,
    SearchWriterResult { name: String },
    SearchTrack,
    SearchTrackResult { artist: String, track: String },
    // View
    ViewMenu,
    ViewLog,
    ViewCreditData,
    ViewTrackData,
    ViewArtistData,
    ViewWriterData,
    // Quiz
    Quiz,
    QuizResult,
    QuizFinal,
}

/// メニュー項目
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub label: String,
    pub screen: Screen,
}

/// アプリケーション状態
pub struct App {
    pub db: Arc<Database>,
    pub config: Config,
    pub running: bool,
    pub mode: Mode,
    pub screen: Screen,
    pub screen_stack: Vec<(Screen, usize, usize, usize)>, // (screen, list_index, list_offset, menu_index)

    // メニュー選択
    pub menu_index: usize,
    pub menu_items: Vec<MenuItem>,

    // リスト表示
    pub list_index: usize,
    pub list_offset: usize,
    pub visible_rows: usize,  // 表示可能な行数（描画時に更新）

    // テキスト入力
    pub input_buffer: String,
    pub input_cursor: usize,
    pub input_label: String,

    // 補完
    pub suggestions: Vec<String>,
    pub suggestion_index: usize,

    // フォーム入力（複数フィールド）
    pub form_fields: Vec<FormField>,
    pub form_index: usize,

    // データ表示
    pub credits: Vec<CreditData>,
    pub artists: Vec<ArtistData>,
    pub writers: Vec<WriterData>,
    pub tracks: Vec<TrackData>,
    pub track_filter: TrackFilter,

    // 検索結果
    pub search_results: Vec<CreditData>,
    pub search_track_data: Option<TrackData>,
    pub search_artist_label: Option<String>,
    pub writer_stats: Vec<(String, i64)>,
    pub writer_yearly: Vec<(String, i64)>,
    pub writer_aoty_count: i64,
    pub writer_soty_count: i64,
    pub writer_total_count: i64,
    pub search_writer_data: Option<WriterData>,
    pub writer_ranks: Vec<(String, i64)>,
    pub writer_data_names: std::collections::HashSet<String>,

    // View内検索
    pub search_query: String,
    pub search_match_indices: Vec<usize>,
    pub search_match_pos: usize, // search_match_indices内の現在位置

    // 入力中のデータ
    pub current_artist: String,
    pub current_track: String,
    // 編集時のSOTY/AOTY保持
    pub edit_is_soty: bool,
    pub edit_is_aoty: bool,

    // メッセージ
    pub message: Option<String>,
    pub error: Option<String>,

    // gg用の待機状態
    pub pending_g: bool,
    // jj用の待機状態（インサートモード）
    pub pending_j: bool,
    // Suggestions選択中（インサートモードでCtrl+p）
    pub selecting_suggestion: bool,

    // ViewArtistData用: undo/redo
    pub artist_undo_stack: Vec<ArtistAction>,
    pub artist_redo_stack: Vec<ArtistAction>,
    // ViewArtistData用: インライン編集
    pub editing_field: Option<usize>,  // 0=Label, 1=Memo
    pub edit_buffer: String,
    pub edit_cursor: usize,

    // ローディング状態
    pub loading: bool,
    pub loading_message: String,
    pub scrape_receiver: Option<mpsc::Receiver<Result<ScrapedSongInfo, String>>>,
    pub tick: usize,

    // songbpm.comバックグラウンドスクレイピング
    pub bpm_receiver: Option<mpsc::Receiver<Result<BpmArtistInfo, String>>>,
    pub bpm_cache: Option<BpmArtistInfo>,
    pub bpm_cache_artist: String,
    pub bpm_matches: Vec<BpmTrackInfo>,
    pub bpm_pending_matches: Vec<BpmTrackInfo>,

    // Spotify再生状態
    pub spotify_playing: bool,
    pub spotify_url: String,
    pub spotify_receiver: Option<mpsc::Receiver<Result<String, String>>>,

    // フォーム上部の表示専用ラベル（j/kで選択不可）
    pub form_header: String,

    // アルバムアートASCII表示
    pub album_art_cache: HashMap<String, crate::scraper::AsciiArt>,
    pub album_art_receiver: Option<mpsc::Receiver<Result<(String, crate::scraper::AsciiArt), String>>>,
    pub album_art_current: Option<crate::scraper::AsciiArt>,

    // WriterAka: 単語一致ペア (name_a, name_b, is_aka)
    pub aka_pairs: Vec<(String, String, bool)>,

    // Around-The-Day Drops: (MM-DD, Vec<(year, artist, track)>)
    pub around_day_drops: Vec<(String, Vec<(String, String, String)>)>,

    // Today's Drops用: MainMenuでロードしたartist/trackを保持
    pub home_artist: String,
    pub home_track: String,
    /// 今日の曲が見つからず、ランダムフォールバックした場合true
    pub is_random_fallback: bool,

    // ViewLog: 削除確認待ち
    pub pending_delete_index: Option<usize>,
    // ViewLog: 削除Undo用スタック
    pub log_undo_stack: Vec<CreditData>,
    // ViewLog: アルバム名インライン編集中
    pub editing_log_album: bool,

    // AutoAdd状態
    pub auto_add_rows: Vec<AutoAddRow>,
    pub auto_add_phase: AutoAddPhase,
    pub auto_add_next_id: u64,
    pub auto_add_rx: Option<mpsc::Receiver<AutoAddMsg>>,
    /// ワーカーへの中断指示。go_to/go_backと Esc で立てる
    pub auto_add_cancel: Arc<AtomicBool>,
    /// インライン編集中のフィールド: 0=Track, 1=Artist
    pub auto_add_editing: Option<usize>,
    /// チェック済み件数 / 全体（進捗表示用）
    pub auto_add_done: usize,
    pub auto_add_total: usize,

    // Quiz状態
    pub quiz_questions: Vec<(String, String, String)>, // (artist, track, spotify_url)
    pub quiz_current: usize,
    pub quiz_score: usize,
    pub quiz_last_correct: bool,
    pub quiz_last_answer: String,
    pub quiz_last_actual: String,
}

/// AutoAdd: 各行のGenius確認状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoAddStatus {
    /// 未チェック
    Pending,
    /// チェック中
    Checking,
    /// Geniusページあり
    Ok,
    /// 全候補URLが404
    NotFound,
    /// ネットワークエラー
    NetError,
    /// 既にcredit_dataにある
    Duplicate,
    /// artist_dataに未登録
    NoArtist,
}

/// AutoAdd: 追加候補1曲分
#[derive(Debug, Clone)]
pub struct AutoAddRow {
    /// 安定ID（配列インデックスではない。結果配送に使う）
    pub id: u64,
    /// 編集のたびに+1。古い結果を捨てるために使う
    pub seq: u32,
    pub track: String,
    pub artist: String,
    /// 表示用（複数アーティスト曲）
    pub artists_all: Vec<String>,
    pub added_at: String,
    pub status: AutoAddStatus,
    /// DBに入れる実際のアーティスト名（Genius名優先で解決したもの）
    pub resolved_artist: Option<String>,
    /// チェック成功時のスクレイプ結果。追加時に再取得しないためのキャッシュ
    pub info: Option<ScrapedSongInfo>,
    /// 実際に当たったGeniusのURL（候補のうちどれが通ったか）
    pub genius_url: Option<String>,
    /// Spotifyから取れたアルバム名（Geniusが拾えなかった場合のフォールバック）
    pub spotify_album: Option<String>,
    /// Spotifyから取れたリリース日（同上）
    pub spotify_date: Option<String>,

    // ✗だったときの値。✓になった後も残して表示する
    pub failed_track: Option<String>,
    pub failed_artist: Option<String>,
    pub failed_url: Option<String>,
}

/// AutoAdd: 画面の進行状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoAddPhase {
    Idle,
    Fetching,
    Checking,
    Ready,
    Empty,
}

/// AutoAdd: ワーカースレッドからのメッセージ
#[derive(Debug)]
pub enum AutoAddMsg {
    /// お気に入り取得完了
    Liked(Result<Vec<crate::spotify::LikedTrack>, String>),
    /// 1行のGenius確認完了
    Checked {
        id: u64,
        seq: u32,
        result: crate::scraper::GeniusCheck,
    },
    /// スキャン全体の中断
    Aborted(String),
}

/// TrackDataフィルタ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackFilter {
    All,
    Soty,
    Aoty,
}

/// ArtistData操作（undo/redo用）
#[derive(Debug, Clone)]
pub enum ArtistAction {
    Move { from: usize, to: usize },
    Delete { index: usize, data: ArtistData },
    Edit { index: usize, old: ArtistData, new: ArtistData },
}

/// フォームフィールド
#[derive(Debug, Clone)]
pub struct FormField {
    pub label: String,
    pub value: String,
    pub cursor: usize,
    /// 選択肢（空ならテキスト入力、非空ならリスト選択型）
    pub options: Vec<String>,
    /// 選択中のインデックス（リスト選択型用）
    pub selected: usize,
}

impl FormField {
    pub fn new(label: &str) -> Self {
        Self {
            label: label.to_string(),
            value: String::new(),
            cursor: 0,
            options: Vec::new(),
            selected: 0,
        }
    }

    pub fn with_value(label: &str, value: &str) -> Self {
        Self {
            label: label.to_string(),
            value: value.to_string(),
            cursor: value.len(),
            options: Vec::new(),
            selected: 0,
        }
    }

    pub fn select(label: &str, options: Vec<&str>, default: usize) -> Self {
        let opts: Vec<String> = options.iter().map(|s| s.to_string()).collect();
        let value = opts.get(default).cloned().unwrap_or_default();
        Self {
            label: label.to_string(),
            value,
            cursor: 0,
            options: opts,
            selected: default,
        }
    }

    pub fn is_select(&self) -> bool {
        !self.options.is_empty()
    }
}

impl App {
    pub fn new(db: Database, config: Config) -> Self {
        let menu_items = vec![
            MenuItem {
                label: "Input".to_string(),
                screen: Screen::InputMenu,
            },
            MenuItem {
                label: "Search".to_string(),
                screen: Screen::SearchMenu,
            },
            MenuItem {
                label: "View".to_string(),
                screen: Screen::ViewMenu,
            },
            MenuItem {
                label: "Quiz".to_string(),
                screen: Screen::Quiz,
            },
        ];

        let mut app = Self {
            db: Arc::new(db),
            config,
            running: true,
            mode: Mode::Normal,
            screen: Screen::MainMenu,
            screen_stack: Vec::new(),
            menu_index: 0,
            menu_items,
            list_index: 0,
            list_offset: 0,
            visible_rows: 20,  // デフォルト値、描画時に更新
            input_buffer: String::new(),
            input_cursor: 0,
            input_label: String::new(),
            suggestions: Vec::new(),
            suggestion_index: 0,
            form_fields: Vec::new(),
            form_index: 0,
            credits: Vec::new(),
            artists: Vec::new(),
            writers: Vec::new(),
            tracks: Vec::new(),
            track_filter: TrackFilter::All,
            search_results: Vec::new(),
            search_track_data: None,
            search_artist_label: None,
            writer_stats: Vec::new(),
            writer_yearly: Vec::new(),
            writer_aoty_count: 0,
            writer_soty_count: 0,
            writer_total_count: 0,
            search_writer_data: None,
            writer_ranks: Vec::new(),
            writer_data_names: std::collections::HashSet::new(),
            search_query: String::new(),
            search_match_indices: Vec::new(),
            search_match_pos: 0,
            current_artist: String::new(),
            current_track: String::new(),
            edit_is_soty: false,
            edit_is_aoty: false,
            message: None,
            error: None,
            pending_g: false,
            pending_j: false,
            selecting_suggestion: false,
            artist_undo_stack: Vec::new(),
            artist_redo_stack: Vec::new(),
            editing_field: None,
            edit_buffer: String::new(),
            edit_cursor: 0,
            loading: false,
            loading_message: String::new(),
            scrape_receiver: None,
            tick: 0,
            bpm_receiver: None,
            bpm_cache: None,
            bpm_cache_artist: String::new(),
            bpm_matches: Vec::new(),
            bpm_pending_matches: Vec::new(),
            spotify_playing: false,
            spotify_url: String::new(),
            spotify_receiver: None,
            form_header: String::new(),
            album_art_cache: HashMap::new(),
            album_art_receiver: None,
            album_art_current: None,
            aka_pairs: Vec::new(),
            around_day_drops: Vec::new(),
            home_artist: String::new(),
            home_track: String::new(),
            is_random_fallback: false,
            pending_delete_index: None,
            log_undo_stack: Vec::new(),
            editing_log_album: false,
            auto_add_rows: Vec::new(),
            auto_add_phase: AutoAddPhase::Idle,
            auto_add_next_id: 0,
            auto_add_rx: None,
            auto_add_cancel: Arc::new(AtomicBool::new(false)),
            auto_add_editing: None,
            auto_add_done: 0,
            auto_add_total: 0,
            quiz_questions: Vec::new(),
            quiz_current: 0,
            quiz_score: 0,
            quiz_last_correct: false,
            quiz_last_answer: String::new(),
            quiz_last_actual: String::new(),
        };
        app.load_screen_data();
        app
    }

    /// 画面を遷移
    /// お気に入り取得をバックグラウンドで開始する。
    /// DBは触らない（rusqliteのConnectionは!Syncなのでスレッドに渡せない）。
    pub fn start_auto_add_fetch(&mut self) {
        let (tx, rx) = mpsc::channel();
        // 前のワーカーが生きていても新しいフラグに差し替えることで巻き添えを防ぐ
        let cancel = Arc::new(AtomicBool::new(false));
        self.auto_add_cancel = Arc::clone(&cancel);
        self.auto_add_rx = Some(rx);
        self.auto_add_phase = AutoAddPhase::Fetching;

        std::thread::spawn(move || {
            let result = crate::spotify::fetch_liked_tracks(&cancel).map_err(|e| e.to_string());
            let _ = tx.send(AutoAddMsg::Liked(result));
        });
    }

    /// 指定した行のGenius確認をバックグラウンドで開始する（チェック／再チェック共通）。
    /// 結果は id+seq で返ってくるので、途中で行を消しても別の行に結果が付かない。
    pub fn start_auto_add_check(&mut self, targets: Vec<(u64, u32, String, String)>) {
        if targets.is_empty() {
            self.auto_add_phase = AutoAddPhase::Ready;
            return;
        }

        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.auto_add_cancel = Arc::clone(&cancel);
        self.auto_add_rx = Some(rx);
        self.auto_add_phase = AutoAddPhase::Checking;
        self.auto_add_done = 0;
        self.auto_add_total = targets.len();

        let config = self.config.clone();

        std::thread::spawn(move || {
            let mut consecutive_net_errors = 0;

            for (id, seq, artist, track) in targets {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }

                let result =
                    crate::scraper::check_genius_candidates(&artist, &track, &config, &cancel);

                // ネットワークが落ちている状態で全件叩き続けても時間の無駄なので、
                // 連続で失敗したらスキャンごと打ち切る
                if matches!(result, crate::scraper::GeniusCheck::NetworkError(_)) {
                    consecutive_net_errors += 1;
                } else {
                    consecutive_net_errors = 0;
                }

                if tx.send(AutoAddMsg::Checked { id, seq, result }).is_err() {
                    return;
                }

                if consecutive_net_errors >= 3 {
                    let _ = tx.send(AutoAddMsg::Aborted(
                        "Network error - check your connection".to_string(),
                    ));
                    return;
                }

                // Geniusに連打しない
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        });
    }

    /// AutoAdd画面から離れるときの後始末。
    /// ワーカーは receiver を落としただけでは止まらないので、必ずキャンセルフラグを立てる。
    fn clear_auto_add(&mut self) {
        self.auto_add_cancel.store(true, Ordering::Relaxed);
        self.auto_add_rx = None;
        self.auto_add_rows.clear();
        self.auto_add_phase = AutoAddPhase::Idle;
        self.auto_add_editing = None;
        self.auto_add_done = 0;
        self.auto_add_total = 0;
    }

    /// AutoAdd: NoArtistの行をDBと照合し直す（Geniusは叩かない。infoはキャッシュ済み）
    pub fn refresh_auto_add_artists(&mut self) {
        for row in self.auto_add_rows.iter_mut() {
            if row.status != AutoAddStatus::NoArtist {
                continue;
            }
            let Some(info) = row.info.as_ref() else {
                continue;
            };
            let genius_artist = crate::scraper::normalize_artist_name(&info.artist);
            let resolved = if self.db.get_artist(&genius_artist).ok().flatten().is_some() {
                Some(genius_artist)
            } else if self.db.get_artist(&row.artist).ok().flatten().is_some() {
                Some(row.artist.clone())
            } else {
                None
            };
            if resolved.is_some() {
                row.status = AutoAddStatus::Ok;
                row.resolved_artist = resolved;
            }
        }
    }

    pub fn go_to(&mut self, screen: Screen) {
        // InputTrackData以外に遷移する場合はBPMキャッシュをクリア
        if !matches!(screen, Screen::InputTrackData) {
            self.bpm_receiver = None;
            self.bpm_cache = None;
            self.bpm_cache_artist.clear();
        }
        // InputAutoAdd以外に遷移する場合はAutoAdd状態クリア＋ワーカー中断。
        // ただしAutoAddから未登録アーティストの登録に行く場合は、戻ってきて続きをやるので残す
        let auto_add_to_artist = matches!(self.screen, Screen::InputAutoAdd)
            && matches!(screen, Screen::InputArtistData);
        if !matches!(screen, Screen::InputAutoAdd) && !auto_add_to_artist {
            self.clear_auto_add();
        }
        // Quiz系以外に遷移する場合はQuiz状態クリア
        if !matches!(screen, Screen::Quiz | Screen::QuizResult | Screen::QuizFinal) {
            self.quiz_questions.clear();
            self.quiz_current = 0;
            self.quiz_score = 0;
        }
        self.screen_stack.push((self.screen.clone(), self.list_index, self.list_offset, self.menu_index));
        self.screen = screen;
        self.menu_index = 0;
        self.list_index = 0;
        self.list_offset = 0;
        self.mode = Mode::Normal;
        self.input_buffer.clear();
        self.input_cursor = 0;
        self.form_fields.clear();
        self.form_header.clear();
        self.form_index = 0;
        self.current_artist.clear();
        self.current_track.clear();
        self.edit_is_soty = false;
        self.edit_is_aoty = false;
        self.search_query.clear();
        self.search_match_indices.clear();
        self.suggestion_index = 0;
        self.message = None;
        self.error = None;
        self.pending_g = false;
        self.pending_j = false;
        self.selecting_suggestion = false;
        self.track_filter = TrackFilter::All;
        self.aka_pairs.clear();
        self.update_menu_items();
        self.load_screen_data();
    }

    /// 前の画面に戻る
    pub fn go_back(&mut self) {
        if let Some((prev, saved_list_index, saved_list_offset, saved_menu_index)) = self.screen_stack.pop() {
            // InputTrackData以外に戻る場合はBPMキャッシュをクリア
            if !matches!(prev, Screen::InputTrackData) {
                self.bpm_receiver = None;
                self.bpm_cache = None;
                self.bpm_cache_artist.clear();
            }
            // InputAutoAdd以外に戻る場合はAutoAdd状態クリア＋ワーカー中断
            if !matches!(prev, Screen::InputAutoAdd) {
                self.clear_auto_add();
            }
            self.screen = prev;
            self.menu_index = saved_menu_index;
            self.list_index = saved_list_index;
            self.list_offset = saved_list_offset;
            self.mode = Mode::Normal;
            self.input_buffer.clear();
            self.form_fields.clear();
            self.form_header.clear();
            self.current_artist.clear();
            self.current_track.clear();
            self.search_query.clear();
            self.search_match_indices.clear();
            self.suggestion_index = 0;
            self.message = None;
            self.error = None;
            self.pending_g = false;
            self.pending_j = false;
            self.selecting_suggestion = false;
            self.track_filter = TrackFilter::All;
            self.update_menu_items();
            self.load_screen_data();
            // load_screen_dataがモードを上書きする場合があるのでNormalに戻す
            self.mode = Mode::Normal;
        }
    }

    /// 現在の画面に応じてメニュー項目を更新
    fn update_menu_items(&mut self) {
        self.menu_items = match &self.screen {
            Screen::MainMenu => vec![
                MenuItem {
                    label: "Input".to_string(),
                    screen: Screen::InputMenu,
                },
                MenuItem {
                    label: "Search".to_string(),
                    screen: Screen::SearchMenu,
                },
                MenuItem {
                    label: "View".to_string(),
                    screen: Screen::ViewMenu,
                },
                MenuItem {
                    label: "Quiz".to_string(),
                    screen: Screen::Quiz,
                },
            ],
            Screen::InputMenu => vec![
                MenuItem {
                    label: "AutoAdd".to_string(),
                    screen: Screen::InputAutoAdd,
                },
                MenuItem {
                    label: "CreditData".to_string(),
                    screen: Screen::InputCreditData,
                },
                MenuItem {
                    label: "TrackData".to_string(),
                    screen: Screen::InputTrackData,
                },
                MenuItem {
                    label: "ArtistData".to_string(),
                    screen: Screen::InputArtistData,
                },
                MenuItem {
                    label: "WriterData".to_string(),
                    screen: Screen::InputWriterData,
                },
                MenuItem {
                    label: "WriterAka".to_string(),
                    screen: Screen::InputWriterAka,
                },
            ],
            Screen::SearchMenu => vec![
                MenuItem {
                    label: "WriterSearch".to_string(),
                    screen: Screen::SearchWriter,
                },
                MenuItem {
                    label: "TrackSearch".to_string(),
                    screen: Screen::SearchTrack,
                },
            ],
            Screen::ViewMenu => vec![
                MenuItem {
                    label: "Log".to_string(),
                    screen: Screen::ViewLog,
                },
                MenuItem {
                    label: "CreditData".to_string(),
                    screen: Screen::ViewCreditData,
                },
                MenuItem {
                    label: "TrackData".to_string(),
                    screen: Screen::ViewTrackData,
                },
                MenuItem {
                    label: "ArtistData".to_string(),
                    screen: Screen::ViewArtistData,
                },
                MenuItem {
                    label: "WriterData".to_string(),
                    screen: Screen::ViewWriterData,
                },
            ],
            _ => Vec::new(),
        };
    }

    /// 画面データをロード
    fn load_screen_data(&mut self) {
        match &self.screen {
            Screen::MainMenu => {
                // 今日のMM-DDに発売された曲からランダムに1つ選ぶ。無ければ全曲からランダム
                let today = chrono::Local::now().format("%m-%d").to_string();
                let today_track = self.db.get_random_track_by_month_day(&today).unwrap_or(None);
                self.is_random_fallback = today_track.is_none();
                let picked = today_track.or_else(|| self.db.get_newest_track().unwrap_or(None));
                if let Some((artist, track)) = picked {
                    self.home_artist = artist.clone();
                    self.home_track = track.clone();
                    self.search_results = self.db.search_song(&artist, &track).unwrap_or_default();
                    self.search_track_data = self.db.get_song_add(&artist, &track).unwrap_or(None);
                    self.search_artist_label = self.db.get_artist(&artist).ok().flatten().and_then(|a| a.label);

                    // Around-The-Day Drops取得（表示曲のリリース日±1日）
                    if let Some(credit) = self.search_results.first() {
                        if let Some(ref date_str) = credit.date {
                            if let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                                let prev = date - chrono::Duration::days(1);
                                let next = date + chrono::Duration::days(1);
                                let md_prev = prev.format("%m-%d").to_string();
                                let md_curr = date.format("%m-%d").to_string();
                                let md_next = next.format("%m-%d").to_string();
                                let album = credit.album.as_deref().unwrap_or("");

                                if let Ok(drops) = self.db.get_around_day_drops(
                                    &md_prev, &md_curr, &md_next, &artist, &track, album,
                                ) {
                                    let mds = [md_prev.clone(), md_curr.clone(), md_next.clone()];
                                    let grouped: Vec<(String, Vec<(String, String, String)>)> = mds
                                        .iter()
                                        .map(|md| {
                                            let items: Vec<(String, String, String)> = drops
                                                .iter()
                                                .filter(|(d, _, _)| d.len() >= 10 && &d[5..] == md.as_str())
                                                .map(|(d, a, t)| (d[..4].to_string(), a.clone(), t.clone()))
                                                .collect();
                                            (md.clone(), items)
                                        })
                                        .collect();
                                    self.around_day_drops = grouped;
                                }
                            }
                        }
                    }

                    // アルバムアート取得
                    if let Some(ref td) = self.search_track_data {
                        if let Some(ref url) = td.spotify {
                            if !url.is_empty() {
                                if let Some(cached) = self.album_art_cache.get(url) {
                                    self.album_art_current = Some(cached.clone());
                                } else {
                                    let url_clone = url.clone();
                                    let (tx, rx) = mpsc::channel();
                                    std::thread::spawn(move || {
                                        let result = crate::scraper::fetch_album_art_ascii(&url_clone, 20, 10);
                                        let _ = tx.send(match result {
                                            Ok(lines) => Ok((url_clone, lines)),
                                            Err(e) => Err(e.to_string()),
                                        });
                                    });
                                    self.album_art_current = None;
                                    self.album_art_receiver = Some(rx);
                                }
                            }
                        }
                    }
                }
            }
            Screen::InputMenu | Screen::SearchMenu | Screen::ViewMenu => {
                // Today's Dropsデータを復元（SearchTrackResultで上書きされている場合）
                if !self.home_artist.is_empty() {
                    let artist = self.home_artist.clone();
                    let track = self.home_track.clone();
                    // 現在のデータが違う曲なら再ロード
                    let current_matches = self.search_track_data.as_ref()
                        .map_or(false, |td| td.artist == artist && td.track == track);
                    if !current_matches {
                        self.search_results = self.db.search_song(&artist, &track).unwrap_or_default();
                        self.search_track_data = self.db.get_song_add(&artist, &track).unwrap_or(None);
                        self.search_artist_label = self.db.get_artist(&artist).ok().flatten().and_then(|a| a.label);

                        // Around-The-Day Drops取得
                        if let Some(credit) = self.search_results.first() {
                            if let Some(ref date_str) = credit.date {
                                if let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                                    let prev = date - chrono::Duration::days(1);
                                    let next = date + chrono::Duration::days(1);
                                    let md_prev = prev.format("%m-%d").to_string();
                                    let md_curr = date.format("%m-%d").to_string();
                                    let md_next = next.format("%m-%d").to_string();
                                    let album = credit.album.as_deref().unwrap_or("");

                                    if let Ok(drops) = self.db.get_around_day_drops(
                                        &md_prev, &md_curr, &md_next, &artist, &track, album,
                                    ) {
                                        let mds = [md_prev.clone(), md_curr.clone(), md_next.clone()];
                                        let grouped: Vec<(String, Vec<(String, String, String)>)> = mds
                                            .iter()
                                            .map(|md| {
                                                let items: Vec<(String, String, String)> = drops
                                                    .iter()
                                                    .filter(|(d, _, _)| d.len() >= 10 && &d[5..] == md.as_str())
                                                    .map(|(d, a, t)| (d[..4].to_string(), a.clone(), t.clone()))
                                                    .collect();
                                                (md.clone(), items)
                                            })
                                            .collect();
                                        self.around_day_drops = grouped;
                                    }
                                }
                            }
                        }

                        // アルバムアート取得
                        if let Some(ref td) = self.search_track_data {
                            if let Some(ref url) = td.spotify {
                                if !url.is_empty() {
                                    if let Some(cached) = self.album_art_cache.get(url) {
                                        self.album_art_current = Some(cached.clone());
                                    } else {
                                        let url_clone = url.clone();
                                        let (tx, rx) = mpsc::channel();
                                        std::thread::spawn(move || {
                                            let result = crate::scraper::fetch_album_art_ascii(&url_clone, 20, 10);
                                            let _ = tx.send(match result {
                                                Ok(lines) => Ok((url_clone, lines)),
                                                Err(e) => Err(e.to_string()),
                                            });
                                        });
                                        self.album_art_current = None;
                                    self.album_art_receiver = Some(rx);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Screen::ViewLog => {
                self.credits = self.db.get_songs_by_log().unwrap_or_default();
                self.writer_data_names = self.db.get_writer_data_names().unwrap_or_default();
            }
            Screen::ViewCreditData => {
                self.credits = self.db.get_songs_sorted().unwrap_or_default();
                self.writer_data_names = self.db.get_writer_data_names().unwrap_or_default();
            }
            Screen::ViewTrackData => {
                self.tracks = self.db.get_all_track_data().unwrap_or_default();
            }
            Screen::ViewArtistData => {
                self.artists = self.db.get_artists_sorted().unwrap_or_default();
            }
            Screen::ViewWriterData => {
                self.writers = self.db.get_writers().unwrap_or_default();
            }
            Screen::InputCreditData => {
                self.form_fields = vec![
                    FormField::new("Artist"),
                    FormField::new("Track"),
                ];
                self.form_index = 0;
                self.mode = Mode::Insert;
                // ArtistDataからアーティスト候補をロード（Trackはサジェスチョンなし）
                self.suggestions = self.db.get_artist_data_names().unwrap_or_default();
                self.suggestion_index = 0;
            }
            Screen::InputArtistData => {
                self.form_fields = vec![
                    FormField::new("Artist"),
                    FormField::new("Label"),
                    FormField::new("Memo"),
                ];
                self.form_index = 0;
                self.mode = Mode::Insert;
                // Labelがないアーティストをサジェスチョン
                self.suggestions = self.db.get_artists_without_label().unwrap_or_default();
                self.suggestion_index = 0;
            }
            Screen::InputWriterData => {
                self.form_fields = vec![
                    FormField::new("Name"),
                    FormField::new("RealName"),
                    FormField::new("BirthDate"),
                    FormField::new("BirthPlace"),
                    FormField::new("Occupation"),
                    FormField::new("Agency"),
                    FormField::new("Debut"),
                    FormField::new("Memo"),
                ];
                self.form_index = 0;
                self.mode = Mode::Insert;
                // Nameフィールド用にライター名をロード
                self.suggestions = self.db.get_credit_names().unwrap_or_default();
                self.suggestion_index = 0;
            }
            Screen::InputAutoAdd if !self.auto_add_rows.is_empty() => {
                // InputArtistDataから戻ってきた: 取り直さず、未登録だった行だけ判定し直す
                self.mode = Mode::Normal;
                self.refresh_auto_add_artists();
            }
            Screen::InputAutoAdd => {
                // 画面に入るたびに取り直す。前回のワーカーは go_to 側で中断済み
                self.auto_add_rows.clear();
                self.auto_add_editing = None;
                self.auto_add_done = 0;
                self.auto_add_total = 0;
                self.list_index = 0;
                self.list_offset = 0;
                self.mode = Mode::Normal;
                self.start_auto_add_fetch();
            }
            Screen::InputWriterAka => {
                self.aka_pairs = compute_aka_pairs(&self.db);
                self.list_index = 0;
                self.list_offset = 0;
                self.mode = Mode::Normal;
            }
            Screen::InputTrackData => {
                // TrackDataがないアーティスト一覧をロード
                if let Ok(artists) = self.db.get_artists_without_add_data() {
                    self.suggestions = artists;
                }
                self.suggestion_index = 0;
                self.input_label = "Artist".to_string();
                self.mode = Mode::Normal;  // Normalモードで選択
            }
            Screen::SearchWriter => {
                self.input_label = "Writer Name".to_string();
                self.mode = Mode::Insert;
            }
            Screen::SearchTrack => {
                if let Ok(artists) = self.db.get_all_artists() {
                    self.suggestions = artists;
                }
                self.suggestion_index = 0;
                self.input_label = "Artist".to_string();
                self.mode = Mode::Insert;
            }
            Screen::SearchWriterResult { name } => {
                self.search_results = self.db.search_songs_by_writer(name).unwrap_or_default();
                self.writer_stats = self.db.get_writer_stats(name).unwrap_or_default();
                self.writer_yearly = self.db.get_writer_yearly_stats(name).unwrap_or_default();
                self.writer_aoty_count = self.db.get_writer_aoty_count(name).unwrap_or(0);
                self.writer_soty_count = self.db.get_writer_soty_count(name).unwrap_or(0);
                self.writer_total_count = self.db.get_writer_total_count(name).unwrap_or(0);
                self.search_writer_data = self.db.get_writer(name).unwrap_or(None);
                self.writer_ranks = self.db.get_writer_ranks(name).unwrap_or_default();
            }
            Screen::Quiz => {
                // 初回のみ問題をロード（QuizResult→Quiz遷移時はリロードしない）
                if self.quiz_questions.is_empty() {
                    self.quiz_questions = self.db.get_random_tracks_with_spotify(10).unwrap_or_default();
                    self.quiz_current = 0;
                    self.quiz_score = 0;
                }
                // SearchTrackと同じUI: Artist入力 → Track選択
                if let Ok(artists) = self.db.get_all_artists() {
                    self.suggestions = artists;
                }
                self.suggestion_index = 0;
                self.input_label = "Artist".to_string();
                self.mode = Mode::Insert;
                // Spotify自動再生はinput::quiz_auto_playから行う（load_screen_data後に呼ぶ）
            }
            Screen::QuizResult | Screen::QuizFinal => {
                // 結果表示画面: データロード不要
            }
            Screen::SearchTrackResult { artist, track } => {
                self.album_art_current = None;
                self.search_results = self.db.search_song(artist, track).unwrap_or_default();
                self.search_track_data = self.db.get_song_add(artist, track).unwrap_or(None);
                self.search_artist_label = self.db.get_artist(artist).ok().flatten().and_then(|a| a.label);
                self.writer_data_names = self.db.get_writer_data_names().unwrap_or_default();

                // Around-The-Day Drops取得
                if let Some(credit) = self.search_results.first() {
                    if let Some(ref date_str) = credit.date {
                        if let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                            let prev = date - chrono::Duration::days(1);
                            let next = date + chrono::Duration::days(1);
                            let md_prev = prev.format("%m-%d").to_string();
                            let md_curr = date.format("%m-%d").to_string();
                            let md_next = next.format("%m-%d").to_string();
                            let album = credit.album.as_deref().unwrap_or("");

                            if let Ok(drops) = self.db.get_around_day_drops(
                                &md_prev, &md_curr, &md_next, artist, track, album,
                            ) {
                                let mds = [md_prev.clone(), md_curr.clone(), md_next.clone()];
                                let grouped: Vec<(String, Vec<(String, String, String)>)> = mds
                                    .iter()
                                    .map(|md| {
                                        let items: Vec<(String, String, String)> = drops
                                            .iter()
                                            .filter(|(d, _, _)| d.len() >= 10 && &d[5..] == md.as_str())
                                            .map(|(d, a, t)| (d[..4].to_string(), a.clone(), t.clone()))
                                            .collect();
                                        (md.clone(), items)
                                    })
                                    .collect();
                                self.around_day_drops = grouped;
                            }
                        }
                    }
                }

                // アルバムアート取得
                if let Some(ref td) = self.search_track_data {
                    if let Some(ref url) = td.spotify {
                        if !url.is_empty() {
                            if let Some(cached) = self.album_art_cache.get(url) {
                                self.album_art_current = Some(cached.clone());
                            } else {
                                // バックグラウンドスレッドで取得
                                let url_clone = url.clone();
                                let (tx, rx) = mpsc::channel();
                                std::thread::spawn(move || {
                                    let result = crate::scraper::fetch_album_art_ascii(&url_clone, 20, 10);
                                    let _ = tx.send(match result {
                                        Ok(lines) => Ok((url_clone, lines)),
                                        Err(e) => Err(e.to_string()),
                                    });
                                });
                                self.album_art_current = None;
                                self.album_art_receiver = Some(rx);
                            }
                        }
                    }
                }
            }
        }
    }

    /// リストの長さを取得
    pub fn list_len(&self) -> usize {
        match &self.screen {
            Screen::ViewLog | Screen::ViewCreditData => self.credits.len(),
            Screen::ViewTrackData => self.tracks.len(),
            Screen::ViewArtistData => self.artists.len(),
            Screen::ViewWriterData => self.writers.len(),
            Screen::InputWriterAka => self.aka_pairs.len(),
            Screen::InputAutoAdd => self.auto_add_rows.len(),
            Screen::SearchWriterResult { .. } | Screen::SearchTrackResult { .. } => {
                self.search_results.len()
            }
            Screen::Quiz => self.suggestions.len(),
            _ => self.menu_items.len(),
        }
    }

    /// メッセージを表示
    pub fn show_message(&mut self, msg: &str) {
        self.message = Some(msg.to_string());
        self.error = None;
    }

    /// エラーを表示
    pub fn show_error(&mut self, err: &str) {
        self.error = Some(err.to_string());
        self.message = None;
    }

    /// メッセージをクリア
    pub fn clear_message(&mut self) {
        self.message = None;
        self.error = None;
    }
}

/// WriterAka用: 単語一致でエイリアス候補ペアを自動検出
fn compute_aka_pairs(db: &Database) -> Vec<(String, String, bool)> {
    let names = db.get_credit_names().unwrap_or_default();

    // 各名前から単語を抽出してインデックス構築
    let mut word_to_names: HashMap<String, Vec<String>> = HashMap::new();
    for name in &names {
        for word in extract_name_words(name) {
            let lower = word.to_lowercase();
            let entry = word_to_names.entry(lower).or_default();
            if !entry.contains(name) {
                entry.push(name.clone());
            }
        }
    }

    // 同じ単語を共有するペアを生成
    let mut pairs: Vec<(String, String)> = Vec::new();
    for group in word_to_names.values() {
        if group.len() < 2 { continue; }
        for i in 0..group.len() {
            for j in (i + 1)..group.len() {
                let (a, b) = if group[i] < group[j] {
                    (group[i].clone(), group[j].clone())
                } else {
                    (group[j].clone(), group[i].clone())
                };
                pairs.push((a, b));
            }
        }
    }
    pairs.sort();
    pairs.dedup();

    // 非表示ペアを除外し、Aka状態を確認してAkaペアを先頭にソート
    let mut result: Vec<(String, String, bool)> = pairs
        .into_iter()
        .filter(|(a, b)| !db.is_aka_dismissed(a, b))
        .map(|(a, b)| {
            let is_aka = db.is_aka_pair(&a, &b).unwrap_or(false);
            (a, b, is_aka)
        })
        .collect();
    result.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
    result
}

/// 名前から単語を抽出（括弧・スペース・カンマで分割、2文字以上）
fn extract_name_words(name: &str) -> Vec<String> {
    const IGNORE_WORDS: &[&str] = &["kor", "jpn", "eng", "chn", "ver", "feat", "remix", "inst"];
    name.split(|c: char| c == '(' || c == ')' || c == ' ' || c == ',')
        .map(|s| s.trim())
        .filter(|s| s.chars().count() >= 2)
        .filter(|s| !IGNORE_WORDS.contains(&s.to_lowercase().as_str()))
        .map(|s| s.to_string())
        .collect()
}
