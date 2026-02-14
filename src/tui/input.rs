use std::sync::mpsc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::models::{ArtistData, BpmArtistInfo, BpmTrackInfo, CreditData, ScrapedSongInfo, TrackData, WriterData};
use crate::scraper::{find_all_tracks_in_bpm_data, make_songbpm_url, make_url, scrape_genius, scrape_songbpm};
use crate::tui::app::{App, FormField, Mode, Screen, TrackFilter};

/// キー入力を処理
pub fn handle_key(app: &mut App, key: KeyEvent) {
    // エラー表示中は任意キーでクリア
    if app.error.is_some() {
        app.clear_message();
        return;
    }
    // メッセージは次の操作時に自動クリア
    app.message = None;

    match app.mode {
        Mode::Normal => handle_normal_mode(app, key),
        Mode::Insert => handle_insert_mode(app, key),
        Mode::Search => handle_search_mode(app, key),
        Mode::Visual => handle_visual_mode(app, key),
    }
}

/// ノーマルモード
fn handle_normal_mode(app: &mut App, key: KeyEvent) {
    // gg コマンドの処理
    if app.pending_g {
        app.pending_g = false;
        if key.code == KeyCode::Char('g') {
            // gg: 先頭に移動
            app.list_index = 0;
            app.list_offset = 0;
            app.menu_index = 0;
            return;
        }
    }

    // ViewLog: 削除確認待ち (y/n)
    if let Some(idx) = app.pending_delete_index {
        match key.code {
            KeyCode::Char('y') => {
                if let Some(credit) = app.credits.get(idx).cloned() {
                    if let Some(id) = credit.id {
                        if let Err(e) = app.db.delete_credit_by_id(id) {
                            app.show_error(&format!("Delete failed: {}", e));
                        } else {
                            app.credits.remove(idx);
                            app.log_undo_stack.push(credit);
                            if app.list_index >= app.credits.len() && app.list_index > 0 {
                                app.list_index -= 1;
                            }
                            app.show_message("Deleted");
                        }
                    }
                }
                app.pending_delete_index = None;
            }
            _ => {
                app.pending_delete_index = None;
                app.show_message("Cancelled");
            }
        }
        return;
    }

    match key.code {
        // アプリ終了
        KeyCode::Char('Q') => {
            stop_spotify(app);
            app.running = false;
        }
        // メニューに戻る
        KeyCode::Char('q') => {
            if app.screen != Screen::MainMenu {
                app.screen_stack.clear();
                app.go_to(Screen::MainMenu);
            }
        }
        // 戻る
        KeyCode::Esc => {
            if app.screen == Screen::MainMenu {
                // MainMenuではEscで何もしない
            } else {
                app.go_back();
            }
        }

        // 移動 (vim style)
        KeyCode::Char('j') | KeyCode::Down => {
            move_down(app);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            move_up(app);
        }

        // インサートモードに入る / メニュー選択
        KeyCode::Char('l') => {
            handle_enter_insert_or_select(app);
        }
        KeyCode::Right => {
            if app.spotify_playing {
                send_spotify_command("forward");
            } else {
                handle_enter_insert_or_select(app);
            }
        }

        // 確定（フォーム送信など）
        KeyCode::Enter => {
            handle_select(app);
        }

        // 戻る
        KeyCode::Left => {
            if app.spotify_playing {
                send_spotify_command("backward");
            } else if matches!(app.screen, Screen::InputTrackData) && !app.current_artist.is_empty() && app.form_fields.is_empty() {
                app.current_artist.clear();
                app.suggestion_index = 0;
                app.input_label = "Artist".to_string();
                if let Ok(artists) = app.db.get_artists_without_add_data() {
                    app.suggestions = artists;
                }
            } else if app.screen != Screen::MainMenu {
                app.go_back();
            }
        }
        KeyCode::Char('h') => {
            if matches!(app.screen, Screen::InputTrackData) && !app.current_artist.is_empty() && app.form_fields.is_empty() {
                // InputTrackDataでTrack選択中 → Artist選択に戻る
                app.current_artist.clear();
                app.suggestion_index = 0;
                app.input_label = "Artist".to_string();
                if let Ok(artists) = app.db.get_artists_without_add_data() {
                    app.suggestions = artists;
                }
            } else if app.screen != Screen::MainMenu {
                app.go_back();
            }
        }

        // 先頭に移動 (gg)
        KeyCode::Char('g') => {
            app.pending_g = true;
        }

        // 末尾に移動
        KeyCode::Char('G') => {
            let len = app.list_len();
            if len > 0 {
                app.list_index = len - 1;
                app.menu_index = app.menu_items.len().saturating_sub(1);
                // list_offsetも更新（カーソルが見えるように）
                if app.list_index >= app.visible_rows {
                    app.list_offset = app.list_index - app.visible_rows + 1;
                }
            }
        }

        // 半ページ下
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = app.visible_rows / 2;
            for _ in 0..half {
                move_down(app);
            }
        }

        // 半ページ上
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = app.visible_rows / 2;
            for _ in 0..half {
                move_up(app);
            }
        }

        // 検索モード
        KeyCode::Char('/') => {
            app.mode = Mode::Search;
            app.input_buffer.clear();
            app.input_cursor = 0;
        }

        // 次の検索ヒット
        KeyCode::Char('n') => {
            search_next(app);
        }

        // 前の検索ヒット
        KeyCode::Char('N') => {
            search_prev(app);
        }

        // a: AOTYフィルタ (ViewTrackData) / 次のBPMマッチ (InputTrackData)
        KeyCode::Char('a') => {
            if matches!(app.screen, Screen::ViewTrackData) {
                app.track_filter = if app.track_filter == TrackFilter::Aoty {
                    TrackFilter::All
                } else {
                    TrackFilter::Aoty
                };
                app.list_index = 0;
                app.list_offset = 0;
                reload_tracks(app);
            } else if matches!(app.screen, Screen::InputTrackData)
                && !app.form_fields.is_empty()
                && app.bpm_matches.len() > 1
            {
                app.suggestion_index = (app.suggestion_index + 1) % app.bpm_matches.len();
                fill_form_from_bpm_match(app);
            }
        }

        // s: SOTYフィルタ (ViewTrackData)
        KeyCode::Char('s') => {
            if matches!(app.screen, Screen::ViewTrackData) {
                app.track_filter = if app.track_filter == TrackFilter::Soty {
                    TrackFilter::All
                } else {
                    TrackFilter::Soty
                };
                app.list_index = 0;
                app.list_offset = 0;
                reload_tracks(app);
            }
        }

        // AOTYトグル
        KeyCode::Char('A') => {
            if matches!(app.screen, Screen::ViewTrackData) {
                if let Some(item) = app.tracks.get(app.list_index) {
                    let artist = item.artist.clone();
                    let track = item.track.clone();
                    match app.db.toggle_aoty(&artist, &track) {
                        Ok(is_aoty) => {
                            let status = if is_aoty { "ON" } else { "OFF" };
                            app.show_message(&format!("AOTY {} for '{}'", status, track));
                            reload_tracks(app);
                        }
                        Err(e) => {
                            app.show_error(&format!("Failed: {}", e));
                        }
                    }
                }
            }
        }

        // SOTYトグル
        KeyCode::Char('S') => {
            handle_space(app);
        }

        // ViewLog: 削除 (y/nで確認)
        KeyCode::Char('d') => {
            if matches!(app.screen, Screen::ViewLog) && !app.credits.is_empty() {
                let c = &app.credits[app.list_index];
                app.show_message(&format!("Delete '{} - {}'? (y/n)", c.artist, c.track));
                app.pending_delete_index = Some(app.list_index);
            }
        }

        // Redo (ViewArtistData)
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if matches!(app.screen, Screen::ViewArtistData) {
                handle_artist_redo(app);
            }
        }

        // ViewLog: リワインド（削除を元に戻す）
        KeyCode::Char('r') => {
            if matches!(app.screen, Screen::ViewLog) {
                if let Some(credit) = app.log_undo_stack.pop() {
                    match app.db.insert_song(&credit) {
                        Ok(_) => {
                            app.show_message(&format!("Restored '{} - {}'", credit.artist, credit.track));
                            app.credits = app.db.get_songs_by_log().unwrap_or_default();
                        }
                        Err(e) => {
                            app.show_error(&format!("Restore failed: {}", e));
                            app.log_undo_stack.push(credit);
                        }
                    }
                } else {
                    app.show_message("Nothing to rewind");
                }
            }
        }

        // URLを開く
        KeyCode::Char('o') => {
            handle_open_url(app);
        }

        // Spotify再生
        KeyCode::Char('c') => {
            handle_open_spotify(app);
        }

        // Spotify停止（プロセスは殺さない）
        KeyCode::Char('x') => {
            if app.spotify_playing {
                send_spotify_command("toggle");
                app.spotify_playing = false;
                app.show_message("Paused");
            }
        }

        // Visualモードに入る (ViewArtistData)
        KeyCode::Char('v') => {
            if matches!(app.screen, Screen::ViewArtistData) && !app.artists.is_empty() {
                app.mode = Mode::Visual;
            }
        }

        // WriterAka: スペースでAkaトグル
        KeyCode::Char(' ') => {
            if matches!(app.screen, Screen::InputWriterAka) {
                handle_writer_aka_toggle(app);
            }
        }

        // インサートモードに入る
        KeyCode::Char('i') => {
            if matches!(
                app.screen,
                Screen::InputCreditData
                    | Screen::InputArtistData
                    | Screen::InputWriterData
                    | Screen::InputTrackData
                    | Screen::SearchWriter
                    | Screen::SearchTrack
            ) {
                app.mode = Mode::Insert;
            }
        }

        // View → Input に遷移（編集）
        KeyCode::Char('e') => {
            if matches!(app.screen, Screen::ViewArtistData) && !app.artists.is_empty() {
                let artist = app.artists[app.list_index].clone();
                app.go_to(Screen::InputArtistData);
                app.form_fields[0].value = artist.artist.clone();
                app.form_fields[0].cursor = artist.artist.chars().count();
                app.form_fields[1].value = artist.label.clone().unwrap_or_default();
                app.form_fields[1].cursor = app.form_fields[1].value.chars().count();
                app.form_fields[2].value = artist.memo.clone().unwrap_or_default();
                app.form_fields[2].cursor = app.form_fields[2].value.chars().count();
                app.form_index = 1; // Labelにカーソル
            } else if matches!(app.screen, Screen::ViewWriterData) && !app.writers.is_empty() {
                let w = app.writers[app.list_index].clone();
                app.go_to(Screen::InputWriterData);
                let vals = [
                    w.name,
                    w.real_name.unwrap_or_default(),
                    w.birth_date.unwrap_or_default(),
                    w.birth_place.unwrap_or_default(),
                    w.occupation.unwrap_or_default(),
                    w.agency.unwrap_or_default(),
                    w.debut.unwrap_or_default(),
                    w.memo.unwrap_or_default(),
                ];
                for (i, val) in vals.iter().enumerate() {
                    if i < app.form_fields.len() {
                        app.form_fields[i].value = val.clone();
                        app.form_fields[i].cursor = val.chars().count();
                    }
                }
                app.form_index = 1; // RealNameにカーソル
            } else if matches!(app.screen, Screen::ViewTrackData) && !app.tracks.is_empty() {
                let t = app.tracks[app.list_index].clone();
                app.go_to(Screen::InputTrackData);
                // アーティスト・トラックを設定してフォーム画面へ直接遷移
                app.current_artist = t.artist.clone();
                app.current_track = t.track.clone();
                let dur = t.duration.map(|d| d.to_string()).unwrap_or_default();
                let spotify = t.spotify.unwrap_or_default();
                let release_idx = if t.is_title { 1 } else if t.is_prerelease { 2 } else { 0 };
                let bpm_field = if let Some(ref b) = t.bpm {
                    if let Ok(n) = b.parse::<i64>() {
                        let half = n / 2;
                        let double = n * 2;
                        FormField::select(
                            "BPM",
                            vec![&half.to_string(), &b, &double.to_string(), "MIXX"],
                            1,
                        )
                    } else {
                        FormField::with_value("BPM", b)
                    }
                } else {
                    FormField::new("BPM")
                };
                app.form_fields = vec![
                    FormField::with_value("Duration (sec)", &dur),
                    bpm_field,
                    FormField::with_value("Spotify URL", &spotify),
                    FormField::select("Release", vec!["-", "Title", "Pre"], release_idx),
                ];
                app.form_index = 0;
                app.mode = Mode::Normal;
            }
        }

        // Undo (ViewArtistData)
        KeyCode::Char('u') => {
            if matches!(app.screen, Screen::ViewArtistData) {
                handle_artist_undo(app);
            }
        }

        _ => {}
    }
}

/// インサートモード
fn handle_insert_mode(app: &mut App, key: KeyEvent) {
    // ViewArtistDataのインライン編集
    if matches!(app.screen, Screen::ViewArtistData) && app.editing_field.is_some() {
        handle_artist_inline_edit(app, key);
        return;
    }

    // Suggestions選択中の処理
    if app.selecting_suggestion {
        handle_suggestion_select(app, key);
        return;
    }

    // リスト選択型フィールドの処理
    if !app.form_fields.is_empty() && app.form_fields[app.form_index].is_select() {
        handle_select_field(app, key);
        return;
    }

    // jj でノーマルモードに戻る
    if key.code == KeyCode::Char('j') {
        if app.pending_j {
            // jj: 直前の'j'を削除してノーマルモードへ
            app.pending_j = false;
            if !app.form_fields.is_empty() {
                let field = &mut app.form_fields[app.form_index];
                if field.cursor > 0 {
                    field.cursor -= 1;
                    remove_char_at(&mut field.value, field.cursor);
                }
            } else if app.input_cursor > 0 {
                app.input_cursor -= 1;
                remove_char_at(&mut app.input_buffer, app.input_cursor);
            }
            app.mode = Mode::Normal;
            app.suggestions.clear();
            return;
        } else {
            // 最初の'j': フラグを立てて通常の文字入力として処理
            app.pending_j = true;
        }
    } else {
        app.pending_j = false;
    }

    match key.code {
        // ノーマルモードに戻る
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.suggestions.clear();
        }

        // キャンセル
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.mode = Mode::Normal;
            app.input_buffer.clear();
            app.suggestions.clear();
            app.go_back();
        }

        // Tab: Suggestions選択モードに入る
        KeyCode::Tab => {
            if !app.suggestions.is_empty() {
                app.selecting_suggestion = true;
            }
        }

        // 前のフィールド
        KeyCode::BackTab => {
            handle_tab(app, true);
        }

        // 文字入力
        KeyCode::Char(c) => {
            if !app.form_fields.is_empty() {
                let field = &mut app.form_fields[app.form_index];
                let byte_idx = char_to_byte_index(&field.value, field.cursor);
                field.value.insert(byte_idx, c);
                field.cursor += 1;
            } else {
                let byte_idx = char_to_byte_index(&app.input_buffer, app.input_cursor);
                app.input_buffer.insert(byte_idx, c);
                app.input_cursor += 1;
            }
            update_suggestions(app);
        }

        // バックスペース
        KeyCode::Backspace => {
            if !app.form_fields.is_empty() {
                let field = &mut app.form_fields[app.form_index];
                if field.cursor > 0 {
                    field.cursor -= 1;
                    remove_char_at(&mut field.value, field.cursor);
                }
            } else if app.input_cursor > 0 {
                app.input_cursor -= 1;
                remove_char_at(&mut app.input_buffer, app.input_cursor);
            }
            update_suggestions(app);
        }

        // カーソル移動
        KeyCode::Left => {
            if !app.form_fields.is_empty() {
                let field = &mut app.form_fields[app.form_index];
                field.cursor = field.cursor.saturating_sub(1);
            } else {
                app.input_cursor = app.input_cursor.saturating_sub(1);
            }
        }

        KeyCode::Right => {
            if !app.form_fields.is_empty() {
                let field = &mut app.form_fields[app.form_index];
                if field.cursor < field.value.chars().count() {
                    field.cursor += 1;
                }
            } else if app.input_cursor < app.input_buffer.chars().count() {
                app.input_cursor += 1;
            }
        }

        KeyCode::Up => {
            // フォームで上のフィールドに移動
            if !app.form_fields.is_empty() && app.form_index > 0 {
                app.form_index -= 1;
                update_suggestions(app);
            } else if !app.suggestions.is_empty() {
                app.suggestion_index = app.suggestion_index.saturating_sub(1);
            }
        }

        KeyCode::Down => {
            // フォームで下のフィールドに移動
            if !app.form_fields.is_empty() && app.form_index < app.form_fields.len() - 1 {
                app.form_index += 1;
                update_suggestions(app);
            } else if !app.suggestions.is_empty() {
                if app.suggestion_index < app.suggestions.len() - 1 {
                    app.suggestion_index += 1;
                }
            }
        }

        _ => {}
    }
}

/// 検索モード
fn handle_search_mode(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.input_buffer.clear();
            app.search_query.clear();
            app.search_match_indices.clear();
        }

        KeyCode::Enter => {
            let query = app.input_buffer.clone();
            if !query.is_empty() {
                app.search_query = query.to_lowercase();
                execute_search(app);
                // 最初のヒットにジャンプ
                if !app.search_match_indices.is_empty() {
                    app.search_match_pos = 0;
                    app.list_index = app.search_match_indices[0];
                }
            }
            app.mode = Mode::Normal;
            app.input_buffer.clear();
        }

        KeyCode::Backspace => {
            app.input_buffer.pop();
            app.input_cursor = app.input_buffer.chars().count();
        }

        KeyCode::Char(c) => {
            app.input_buffer.push(c);
            app.input_cursor = app.input_buffer.chars().count();
        }

        _ => {}
    }
}

/// 検索を実行してマッチするインデックスを収集
fn execute_search(app: &mut App) {
    let q = &app.search_query;
    app.search_match_indices.clear();

    match &app.screen {
        Screen::ViewCreditData | Screen::ViewLog => {
            for (i, c) in app.credits.iter().enumerate() {
                if c.artist.to_lowercase().contains(q)
                    || c.track.to_lowercase().contains(q)
                    || c.name.as_deref().unwrap_or("").to_lowercase().contains(q)
                    || c.album.as_deref().unwrap_or("").to_lowercase().contains(q)
                {
                    app.search_match_indices.push(i);
                }
            }
        }
        Screen::ViewTrackData => {
            for (i, t) in app.tracks.iter().enumerate() {
                if t.artist.to_lowercase().contains(q)
                    || t.track.to_lowercase().contains(q)
                    || t.album.as_deref().unwrap_or("").to_lowercase().contains(q)
                {
                    app.search_match_indices.push(i);
                }
            }
        }
        Screen::ViewArtistData => {
            for (i, a) in app.artists.iter().enumerate() {
                if a.artist.to_lowercase().contains(q)
                    || a.label.as_deref().unwrap_or("").to_lowercase().contains(q)
                {
                    app.search_match_indices.push(i);
                }
            }
        }
        Screen::ViewWriterData => {
            for (i, w) in app.writers.iter().enumerate() {
                if w.name.to_lowercase().contains(q)
                    || w.real_name.as_deref().unwrap_or("").to_lowercase().contains(q)
                    || w.occupation.as_deref().unwrap_or("").to_lowercase().contains(q)
                {
                    app.search_match_indices.push(i);
                }
            }
        }
        Screen::SearchWriterResult { .. } | Screen::SearchTrackResult { .. } => {
            for (i, c) in app.search_results.iter().enumerate() {
                if c.artist.to_lowercase().contains(q)
                    || c.track.to_lowercase().contains(q)
                    || c.name.as_deref().unwrap_or("").to_lowercase().contains(q)
                {
                    app.search_match_indices.push(i);
                }
            }
        }
        _ => {}
    }
}

/// 次の検索ヒットに移動
fn search_next(app: &mut App) {
    if app.search_match_indices.is_empty() { return; }
    app.search_match_pos = (app.search_match_pos + 1) % app.search_match_indices.len();
    app.list_index = app.search_match_indices[app.search_match_pos];
}

/// 前の検索ヒットに移動
fn search_prev(app: &mut App) {
    if app.search_match_indices.is_empty() { return; }
    if app.search_match_pos == 0 {
        app.search_match_pos = app.search_match_indices.len() - 1;
    } else {
        app.search_match_pos -= 1;
    }
    app.list_index = app.search_match_indices[app.search_match_pos];
}

/// Visualモード（ViewArtistData用: 並び替え・削除）
fn handle_visual_mode(app: &mut App, key: KeyEvent) {
    use crate::tui::app::ArtistAction;

    match key.code {
        // Normalモードに戻る（DBに保存）
        KeyCode::Esc | KeyCode::Char('v') => {
            // 変更をDBに保存
            if let Err(e) = app.db.update_artists_order(&app.artists) {
                app.show_error(&format!("Failed to save: {}", e));
            } else {
                app.show_message("Changes saved");
            }
            app.mode = Mode::Normal;
            app.artist_undo_stack.clear();
            app.artist_redo_stack.clear();
        }

        // 上に移動（アイテムを上に移動）
        KeyCode::Char('k') | KeyCode::Up => {
            if app.list_index > 0 {
                // スワップ
                app.artists.swap(app.list_index, app.list_index - 1);
                // undoスタックに追加
                app.artist_undo_stack.push(ArtistAction::Move {
                    from: app.list_index,
                    to: app.list_index - 1,
                });
                app.artist_redo_stack.clear();
                app.list_index -= 1;
                // スクロール調整
                if app.list_index < app.list_offset {
                    app.list_offset = app.list_index;
                }
            }
        }

        // 下に移動（アイテムを下に移動）
        KeyCode::Char('j') | KeyCode::Down => {
            if app.list_index < app.artists.len() - 1 {
                // スワップ
                app.artists.swap(app.list_index, app.list_index + 1);
                // undoスタックに追加
                app.artist_undo_stack.push(ArtistAction::Move {
                    from: app.list_index,
                    to: app.list_index + 1,
                });
                app.artist_redo_stack.clear();
                app.list_index += 1;
                // スクロール調整
                if app.list_index >= app.list_offset + app.visible_rows {
                    app.list_offset = app.list_index - app.visible_rows + 1;
                }
            }
        }

        // 削除
        KeyCode::Char('d') => {
            if !app.artists.is_empty() {
                let removed = app.artists.remove(app.list_index);
                // DBからも削除
                if let Err(e) = app.db.delete_artist(&removed.artist) {
                    app.show_error(&format!("Failed to delete: {}", e));
                    // 削除失敗時は元に戻す
                    app.artists.insert(app.list_index, removed);
                    return;
                }
                app.artist_undo_stack.push(ArtistAction::Delete {
                    index: app.list_index,
                    data: removed,
                });
                app.artist_redo_stack.clear();
                // インデックス調整
                if app.list_index >= app.artists.len() && app.list_index > 0 {
                    app.list_index -= 1;
                }
                if app.artists.is_empty() {
                    app.mode = Mode::Normal;
                }
            }
        }

        // Undo
        KeyCode::Char('u') => {
            handle_artist_undo(app);
        }

        // Redo
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            handle_artist_redo(app);
        }

        _ => {}
    }
}

/// フィルタに応じてTrackDataリストを再取得
fn reload_tracks(app: &mut App) {
    app.tracks = match app.track_filter {
        TrackFilter::Soty => app.db.get_soty().unwrap_or_default(),
        TrackFilter::Aoty => app.db.get_aoty().unwrap_or_default(),
        TrackFilter::All => app.db.get_all_track_data().unwrap_or_default(),
    };
}

/// Artist操作のUndo
fn handle_artist_undo(app: &mut App) {
    use crate::tui::app::ArtistAction;

    if let Some(action) = app.artist_undo_stack.pop() {
        match action.clone() {
            ArtistAction::Move { from, to } => {
                // 逆方向にスワップ
                app.artists.swap(from, to);
                app.list_index = from;
                app.artist_redo_stack.push(action);
            }
            ArtistAction::Delete { index, data } => {
                // 削除を元に戻す（DBにも再挿入）
                if let Err(e) = app.db.upsert_artist(&data) {
                    app.show_error(&format!("Failed to restore: {}", e));
                    app.artist_undo_stack.push(action);
                    return;
                }
                app.artists.insert(index, data.clone());
                app.list_index = index;
                app.artist_redo_stack.push(action);
            }
            ArtistAction::Edit { index, old, new: _ } => {
                // 編集を元に戻す（DBにも保存）
                if index < app.artists.len() {
                    if let Err(e) = app.db.upsert_artist(&old) {
                        app.show_error(&format!("Failed to undo: {}", e));
                        app.artist_undo_stack.push(action);
                        return;
                    }
                    let current = app.artists[index].clone();
                    app.artists[index] = old;
                    app.artist_redo_stack.push(ArtistAction::Edit {
                        index,
                        old: current,
                        new: app.artists[index].clone(),
                    });
                }
            }
        }
    }
}

/// Artist操作のRedo
fn handle_artist_redo(app: &mut App) {
    use crate::tui::app::ArtistAction;

    if let Some(action) = app.artist_redo_stack.pop() {
        match action.clone() {
            ArtistAction::Move { from, to } => {
                // 再度スワップ
                app.artists.swap(from, to);
                app.list_index = to;
                app.artist_undo_stack.push(action);
            }
            ArtistAction::Delete { index, data } => {
                // 再度削除（DBからも削除）
                if index < app.artists.len() {
                    if let Err(e) = app.db.delete_artist(&data.artist) {
                        app.show_error(&format!("Failed to delete: {}", e));
                        app.artist_redo_stack.push(action);
                        return;
                    }
                    app.artists.remove(index);
                    app.artist_undo_stack.push(ArtistAction::Delete { index, data });
                    if app.list_index >= app.artists.len() && app.list_index > 0 {
                        app.list_index -= 1;
                    }
                }
            }
            ArtistAction::Edit { index, old, new } => {
                // 再度編集（DBにも保存）
                if index < app.artists.len() {
                    if let Err(e) = app.db.upsert_artist(&new) {
                        app.show_error(&format!("Failed to redo: {}", e));
                        app.artist_redo_stack.push(action);
                        return;
                    }
                    app.artists[index] = new.clone();
                    app.artist_undo_stack.push(ArtistAction::Edit { index, old, new });
                }
            }
        }
    }
}

/// ViewArtistDataのインライン編集
fn handle_artist_inline_edit(app: &mut App, key: KeyEvent) {
    // jj でノーマルモードに戻る（編集を確定）
    if key.code == KeyCode::Char('j') {
        if app.pending_j {
            app.pending_j = false;
            // 直前の'j'を削除
            if app.edit_cursor > 0 {
                app.edit_cursor -= 1;
                remove_char_at(&mut app.edit_buffer, app.edit_cursor);
            }
            // 編集を確定してundoスタックに追加
            save_artist_edit(app);
            app.mode = Mode::Normal;
            app.editing_field = None;
            return;
        } else {
            app.pending_j = true;
        }
    } else {
        app.pending_j = false;
    }

    match key.code {
        // Escでキャンセル
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.editing_field = None;
            app.edit_buffer.clear();
            app.edit_cursor = 0;
        }

        // Tab/↓で次のフィールドへ
        KeyCode::Tab | KeyCode::Down => {
            // 現在の編集を保存
            save_artist_edit(app);
            // 次のフィールドへ
            if let Some(field) = app.editing_field {
                if field == 0 {
                    // Label -> Memo
                    app.editing_field = Some(1);
                    if let Some(artist) = app.artists.get(app.list_index) {
                        app.edit_buffer = artist.memo.clone().unwrap_or_default();
                        app.edit_cursor = app.edit_buffer.chars().count();
                    }
                } else {
                    // Memo -> 次の行のLabel
                    if app.list_index < app.artists.len() - 1 {
                        app.list_index += 1;
                        app.editing_field = Some(0);
                        if let Some(artist) = app.artists.get(app.list_index) {
                            app.edit_buffer = artist.label.clone().unwrap_or_default();
                            app.edit_cursor = app.edit_buffer.chars().count();
                        }
                    }
                }
            }
        }

        // Shift+Tab/↑で前のフィールドへ
        KeyCode::BackTab | KeyCode::Up => {
            // 現在の編集を保存
            save_artist_edit(app);
            // 前のフィールドへ
            if let Some(field) = app.editing_field {
                if field == 1 {
                    // Memo -> Label
                    app.editing_field = Some(0);
                    if let Some(artist) = app.artists.get(app.list_index) {
                        app.edit_buffer = artist.label.clone().unwrap_or_default();
                        app.edit_cursor = app.edit_buffer.chars().count();
                    }
                } else {
                    // Label -> 前の行のMemo
                    if app.list_index > 0 {
                        app.list_index -= 1;
                        app.editing_field = Some(1);
                        if let Some(artist) = app.artists.get(app.list_index) {
                            app.edit_buffer = artist.memo.clone().unwrap_or_default();
                            app.edit_cursor = app.edit_buffer.chars().count();
                        }
                    }
                }
            }
        }

        // 文字入力
        KeyCode::Char(c) => {
            let byte_idx = char_to_byte_index(&app.edit_buffer, app.edit_cursor);
            app.edit_buffer.insert(byte_idx, c);
            app.edit_cursor += 1;
        }

        // バックスペース
        KeyCode::Backspace => {
            if app.edit_cursor > 0 {
                app.edit_cursor -= 1;
                remove_char_at(&mut app.edit_buffer, app.edit_cursor);
            }
        }

        // カーソル移動
        KeyCode::Left => {
            app.edit_cursor = app.edit_cursor.saturating_sub(1);
        }
        KeyCode::Right => {
            if app.edit_cursor < app.edit_buffer.chars().count() {
                app.edit_cursor += 1;
            }
        }

        _ => {}
    }
}

/// Artist編集の保存
fn save_artist_edit(app: &mut App) {
    use crate::tui::app::ArtistAction;
    if let Some(field) = app.editing_field {
        if let Some(artist) = app.artists.get(app.list_index).cloned() {
            let old = artist.clone();
            let mut new = artist;
            let new_value = if app.edit_buffer.is_empty() {
                None
            } else {
                Some(app.edit_buffer.clone())
            };

            if field == 0 {
                new.label = new_value;
            } else {
                new.memo = new_value;
            }

            // 変更があった場合のみ保存
            if old.label != new.label || old.memo != new.memo {
                // DBに保存
                if let Err(e) = app.db.upsert_artist(&new) {
                    app.show_error(&format!("Failed to save: {}", e));
                    return;
                }
                app.artists[app.list_index] = new.clone();
                app.artist_undo_stack.push(ArtistAction::Edit {
                    index: app.list_index,
                    old,
                    new,
                });
                app.artist_redo_stack.clear();
            }
        }
    }
}

/// Suggestions選択モード（インサートモード中にCtrl+pで入る）
fn handle_suggestion_select(app: &mut App, key: KeyEvent) {
    match key.code {
        // 下に移動
        KeyCode::Char('j') | KeyCode::Down => {
            if !app.suggestions.is_empty() && app.suggestion_index < app.suggestions.len() - 1 {
                app.suggestion_index += 1;
            }
        }

        // 上に移動
        KeyCode::Char('k') | KeyCode::Up => {
            if !app.suggestions.is_empty() {
                app.suggestion_index = app.suggestion_index.saturating_sub(1);
            }
        }

        // 選択確定: 候補を入力欄に反映してノーマルモードに戻る
        KeyCode::Char('l') | KeyCode::Enter => {
            if let Some(suggestion) = app.suggestions.get(app.suggestion_index) {
                if !app.form_fields.is_empty() {
                    app.form_fields[app.form_index].value = suggestion.clone();
                    app.form_fields[app.form_index].cursor = suggestion.chars().count();
                } else {
                    app.input_buffer = suggestion.clone();
                    app.input_cursor = suggestion.chars().count();
                }
            }
            app.selecting_suggestion = false;
            app.mode = Mode::Normal;
        }

        // キャンセル: Suggestions選択モードを抜ける（インサートモードに戻る）
        KeyCode::Char('h') => {
            app.selecting_suggestion = false;
        }

        _ => {}
    }
}

/// 下に移動
fn move_down(app: &mut App) {
    if is_menu_screen(&app.screen) {
        if app.menu_index < app.menu_items.len() - 1 {
            app.menu_index += 1;
        }
    } else if is_in_form_mode(app) {
        // フォーム画面: フィールド間を移動
        if !app.form_fields.is_empty() && app.form_index < app.form_fields.len() - 1 {
            app.form_index += 1;
        }
    } else if is_suggestion_screen(&app.screen) {
        // 補完候補画面: 補完候補間を移動
        if !app.suggestions.is_empty() && app.suggestion_index < app.suggestions.len() - 1 {
            app.suggestion_index += 1;
        }
    } else {
        // View画面などのリスト表示
        let len = app.list_len();
        if len > 0 && app.list_index < len - 1 {
            app.list_index += 1;
            // カーソルが画面下端を超えたらスクロール
            if app.list_index >= app.list_offset + app.visible_rows {
                // カーソルが見えるようにスクロール
                app.list_offset = app.list_index - app.visible_rows + 1;
            }
        }
    }
}

/// 上に移動
fn move_up(app: &mut App) {
    if is_menu_screen(&app.screen) {
        app.menu_index = app.menu_index.saturating_sub(1);
    } else if is_in_form_mode(app) {
        // フォーム画面: フィールド間を移動
        if !app.form_fields.is_empty() {
            app.form_index = app.form_index.saturating_sub(1);
        }
    } else if is_suggestion_screen(&app.screen) {
        // 補完候補画面: 補完候補間を移動
        if !app.suggestions.is_empty() {
            app.suggestion_index = app.suggestion_index.saturating_sub(1);
        }
    } else {
        // View画面などのリスト表示
        if app.list_index > 0 {
            app.list_index -= 1;
            // カーソルが画面上端より上に行ったらスクロール
            if app.list_index < app.list_offset {
                app.list_offset = app.list_index;
            }
        }
    }
}

/// メニュー画面かどうか
fn is_menu_screen(screen: &Screen) -> bool {
    matches!(
        screen,
        Screen::MainMenu | Screen::InputMenu | Screen::SearchMenu | Screen::ViewMenu
    )
}

/// フォーム入力画面かどうか（フォームが表示されている場合）
fn is_form_screen(screen: &Screen) -> bool {
    matches!(
        screen,
        Screen::InputCreditData | Screen::InputArtistData | Screen::InputWriterData
    )
}

/// フォーム入力中かどうか（InputTrackDataでフォーム表示中も含む）
fn is_in_form_mode(app: &App) -> bool {
    if is_form_screen(&app.screen) {
        return true;
    }
    // InputTrackDataでフォームが表示されている場合
    if matches!(app.screen, Screen::InputTrackData) && !app.form_fields.is_empty() {
        return true;
    }
    false
}

/// 補完候補表示画面かどうか
fn is_suggestion_screen(screen: &Screen) -> bool {
    matches!(
        screen,
        Screen::InputTrackData | Screen::SearchTrack | Screen::SearchWriter
    )
}

/// l/→ でインサートモードに入る、またはメニュー選択
fn handle_enter_insert_or_select(app: &mut App) {
    if is_menu_screen(&app.screen) {
        // メニュー画面: 選択して遷移
        if let Some(item) = app.menu_items.get(app.menu_index) {
            let screen = item.screen.clone();
            app.go_to(screen);
        }
    } else if matches!(app.screen, Screen::InputTrackData) && app.form_fields.is_empty() {
        // InputTrackData（フォーム未表示）: Normalモードで選択
        handle_confirm(app);
    } else if matches!(app.screen, Screen::ViewWriterData) && !app.writers.is_empty() {
        // ViewWriterData: SearchWriterResultに遷移
        let name = app.writers[app.list_index].name.clone();
        app.go_to(Screen::SearchWriterResult { name });
    } else if matches!(app.screen, Screen::ViewTrackData) && !app.tracks.is_empty() {
        // ViewTrackData/ViewBest16: SearchTrackResultに遷移
        let t = &app.tracks[app.list_index];
        let artist = t.artist.clone();
        let track = t.track.clone();
        app.go_to(Screen::SearchTrackResult { artist, track });
    } else if matches!(app.screen, Screen::SearchTrackResult { .. }) && !app.search_results.is_empty() {
        // SearchTrackResult: 選択したCreditのnameでSearchWriterResultに遷移
        if let Some(name) = app.search_results.get(app.list_index).and_then(|c| c.name.clone()) {
            if !name.is_empty() {
                app.go_to(Screen::SearchWriterResult { name });
            }
        }
    } else if matches!(app.screen, Screen::ViewLog | Screen::ViewCreditData) && !app.credits.is_empty() {
        // ViewLog/ViewCreditData: 選択したCreditのnameでSearchWriterResultに遷移
        if let Some(name) = app.credits.get(app.list_index).and_then(|c| c.name.clone()) {
            if !name.is_empty() {
                app.go_to(Screen::SearchWriterResult { name });
            }
        }
    } else if matches!(app.screen, Screen::SearchWriterResult { .. }) && !app.search_results.is_empty() {
        // SearchWriterResult: 選択した曲のSearchTrackResultに遷移
        let c = &app.search_results[app.list_index];
        let artist = c.artist.clone();
        let track = c.track.clone();
        app.go_to(Screen::SearchTrackResult { artist, track });
    } else if is_form_screen(&app.screen) || is_suggestion_screen(&app.screen) || is_in_form_mode(app) {
        // フォーム画面・補完候補画面: インサートモードに入る
        app.mode = Mode::Insert;
    }
}

/// 選択を実行（Enter）
fn handle_select(app: &mut App) {
    if is_menu_screen(&app.screen) {
        if let Some(item) = app.menu_items.get(app.menu_index) {
            let screen = item.screen.clone();
            app.go_to(screen);
        }
    } else if is_form_screen(&app.screen) || is_suggestion_screen(&app.screen) {
        // フォーム画面・補完候補画面: 確定処理
        handle_confirm(app);
    } else {
        // リスト選択の処理
        match &app.screen {
            Screen::InputTrackData => {
                // アーティスト選択後、曲選択へ
                if !app.suggestions.is_empty() {
                    if let Some(artist) = app.suggestions.get(app.list_index) {
                        app.current_artist = artist.clone();
                        if let Ok(tracks) = app.db.get_tracks_by_artist(artist) {
                            app.suggestions = tracks;
                            app.list_index = 0;
                            app.input_label = "Track".to_string();
                        }
                    }
                }
            }
            Screen::ViewWriterData => {
                if !app.writers.is_empty() {
                    let name = app.writers[app.list_index].name.clone();
                    app.go_to(Screen::SearchWriterResult { name });
                }
            }
            _ => {}
        }
    }
}

/// 確定処理（ノーマルモードでEnter）
fn handle_confirm(app: &mut App) {
    match &app.screen {
        Screen::InputCreditData => {
            // フォームからデータを取得してスクレイピング（バックグラウンド）
            if app.form_fields.len() >= 2 {
                let artist = app.form_fields[0].value.trim().to_string();
                let track = app.form_fields[1].value.trim().to_string();

                if artist.is_empty() || track.is_empty() {
                    app.show_error("Artist and Track are required");
                    return;
                }

                let url = make_url(&artist, &track);
                let config = app.config.clone();

                let (tx, rx) = mpsc::channel();
                app.loading = true;
                app.loading_message = format!("Fetching: {}", url);
                app.scrape_receiver = Some(rx);

                std::thread::spawn(move || {
                    let result = scrape_genius(&url, &config);
                    let _ = tx.send(result.map_err(|e| e.to_string()));
                });
            }
        }

        Screen::InputArtistData => {
            if app.form_fields.len() >= 3 {
                let artist = app.form_fields[0].value.trim().to_string();
                let label = app.form_fields[1].value.trim().to_string();
                let memo = app.form_fields[2].value.trim().to_string();

                if artist.is_empty() {
                    app.show_error("Artist name is required");
                    return;
                }

                let data = ArtistData {
                    id: None,
                    artist: artist.clone(),
                    label: if label.is_empty() { None } else { Some(label) },
                    memo: if memo.is_empty() { None } else { Some(memo) },
                    sort_order: None,
                };

                match app.db.upsert_artist(&data) {
                    Ok(_) => {
                        app.show_message(&format!("Artist '{}' saved", artist));
                        for field in &mut app.form_fields {
                            field.value.clear();
                            field.cursor = 0;
                        }
                        app.form_index = 0;
                    }
                    Err(e) => {
                        app.show_error(&format!("Failed to save: {}", e));
                    }
                }
            }
        }

        Screen::InputWriterData => {
            if !app.form_fields.is_empty() {
                let name = app.form_fields[0].value.trim().to_string();

                if name.is_empty() {
                    app.show_error("Name is required");
                    return;
                }

                let get_field = |idx: usize| -> Option<String> {
                    app.form_fields
                        .get(idx)
                        .map(|f| f.value.trim().to_string())
                        .filter(|s| !s.is_empty())
                };

                let data = WriterData {
                    id: None,
                    name: name.clone(),
                    real_name: get_field(1),
                    birth_date: get_field(2),
                    birth_place: get_field(3),
                    occupation: get_field(4),
                    agency: get_field(5),
                    debut: get_field(6),
                    memo: get_field(7),
                };

                match app.db.upsert_writer(&data) {
                    Ok(_) => {
                        app.show_message(&format!("Writer '{}' saved", name));
                        for field in &mut app.form_fields {
                            field.value.clear();
                            field.cursor = 0;
                        }
                        app.form_index = 0;
                    }
                    Err(e) => {
                        app.show_error(&format!("Failed to save: {}", e));
                    }
                }
            }
        }

        Screen::SearchWriter => {
            let name = app.input_buffer.trim().to_string();
            if !name.is_empty() {
                app.go_to(Screen::SearchWriterResult { name });
            }
        }

        Screen::SearchTrack => {
            // 2段階入力: まずArtist、次にTrack
            if app.current_artist.is_empty() {
                let artist = app.input_buffer.trim().to_string();
                if !artist.is_empty() {
                    app.current_artist = artist.clone();
                    app.input_buffer.clear();
                    app.input_cursor = 0;
                    app.input_label = "Track".to_string();

                    // 曲の補完候補を更新
                    if let Ok(tracks) = app.db.get_tracks_by_artist(&artist) {
                        app.suggestions = tracks;
                    }
                }
            } else {
                let track = app.input_buffer.trim().to_string();
                if !track.is_empty() {
                    let artist = app.current_artist.clone();
                    app.current_artist.clear();
                    app.go_to(Screen::SearchTrackResult { artist, track });
                }
            }
        }

        Screen::InputTrackData => {
            // フォーム表示中の場合 → DB保存
            if !app.form_fields.is_empty() {
                let duration: Option<i64> = parse_duration(app.form_fields[0].value.trim());
                let bpm_str = app.form_fields[1].value.trim().to_string();
                let bpm: Option<String> = if bpm_str.is_empty() { None } else { Some(bpm_str) };
                let spotify = if app.form_fields[2].value.trim().is_empty() {
                    None
                } else {
                    Some(app.form_fields[2].value.trim().to_string())
                };
                // Releaseフィールドの解析
                let release = app.form_fields.get(3)
                    .map(|f| f.value.trim().to_lowercase())
                    .unwrap_or_default();
                let is_title = release == "title";
                let is_prerelease = release == "pre";

                let data = TrackData {
                    id: None,
                    track: app.current_track.clone(),
                    artist: app.current_artist.clone(),
                    label: None,
                    date: None,
                    album: None,
                    duration,
                    bpm,
                    spotify,
                    is_title,
                    is_prerelease,
                    is_aoty: false,
                    is_soty: false,
                };

                match app.db.upsert_song_add(&data) {
                    Ok(_) => {
                        app.show_message(&format!(
                            "TrackData saved: {} - {}",
                            app.current_artist, app.current_track
                        ));
                        // リセットして次の曲入力へ
                        app.form_fields.clear();
                        app.current_track.clear();
                        app.current_artist.clear();
                        app.suggestion_index = 0;
                        app.mode = Mode::Normal;
                        // 残りのアーティストをリロード
                        if let Ok(artists) = app.db.get_artists_without_add_data() {
                            app.suggestions = artists;
                        }
                        app.input_label = "Artist".to_string();
                    }
                    Err(e) => {
                        app.show_error(&format!("Failed to save: {}", e));
                    }
                }
            } else if !app.suggestions.is_empty() {
                // Suggestionsから選択（Normalモード）
                if app.current_artist.is_empty() {
                    // アーティスト選択
                    if let Some(artist) = app.suggestions.get(app.suggestion_index) {
                        app.current_artist = artist.clone();
                        app.input_label = "Track".to_string();
                        app.suggestion_index = 0;
                        // AddDataがない曲のみロード
                        if let Ok(tracks) = app.db.get_tracks_without_add_data(&app.current_artist) {
                            app.suggestions = tracks;
                        }
                    }
                } else {
                    // 曲選択
                    if let Some(track) = app.suggestions.get(app.suggestion_index) {
                        app.current_track = track.clone();

                        // キャッシュがあれば即マッチ処理、なければスクレイプ開始
                        if app.bpm_cache.is_some() && app.bpm_cache_artist == app.current_artist {
                            handle_bpm_matches(app);
                        } else {
                            start_bpm_scrape(app);
                        }
                    }
                }
            }
        }


        _ => {}
    }
}

/// リスト選択型フィールドの処理
fn handle_select_field(app: &mut App, key: KeyEvent) {
    // jj でノーマルモードに戻る
    if key.code == KeyCode::Char('j') {
        if app.pending_j {
            app.pending_j = false;
            app.mode = Mode::Normal;
            return;
        } else {
            app.pending_j = true;
            return;
        }
    } else {
        app.pending_j = false;
    }

    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
        }

        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.mode = Mode::Normal;
            app.go_back();
        }

        // 左に移動
        KeyCode::Char('h') | KeyCode::Left => {
            let field = &mut app.form_fields[app.form_index];
            if field.selected > 0 {
                field.selected -= 1;
                field.value = field.options[field.selected].clone();
            }
        }

        // 右に移動
        KeyCode::Char('l') | KeyCode::Right => {
            let field = &mut app.form_fields[app.form_index];
            if field.selected < field.options.len() - 1 {
                field.selected += 1;
                field.value = field.options[field.selected].clone();
            }
        }

        // 次/前のフィールド
        KeyCode::Tab => {
            handle_tab(app, false);
        }
        KeyCode::BackTab => {
            handle_tab(app, true);
        }

        _ => {}
    }
}

/// Tab / Shift+Tab
fn handle_tab(app: &mut App, reverse: bool) {
    if !app.suggestions.is_empty() {
        // 補完候補を選択
        if reverse {
            app.suggestion_index = app.suggestion_index.saturating_sub(1);
        } else if app.suggestion_index < app.suggestions.len() - 1 {
            app.suggestion_index += 1;
        }

        // 選択した候補を入力欄に反映
        if let Some(suggestion) = app.suggestions.get(app.suggestion_index) {
            if !app.form_fields.is_empty() {
                app.form_fields[app.form_index].value = suggestion.clone();
                app.form_fields[app.form_index].cursor = suggestion.chars().count();
            } else {
                app.input_buffer = suggestion.clone();
                app.input_cursor = suggestion.chars().count();
            }
        }
    } else if !app.form_fields.is_empty() {
        // フォームフィールド間を移動
        if reverse {
            if app.form_index > 0 {
                app.form_index -= 1;
                update_suggestions(app);
            }
        } else if app.form_index < app.form_fields.len() - 1 {
            app.form_index += 1;
            update_suggestions(app);
        }
    }
}

/// Duration文字列をパース（秒数 or M:SS形式）
fn parse_duration(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    // M:SS 形式
    if let Some((m, ss)) = s.split_once(':') {
        let min: i64 = m.trim().parse().ok()?;
        let sec: i64 = ss.trim().parse().ok()?;
        return Some(min * 60 + sec);
    }
    // 秒数
    s.parse().ok()
}

/// 補完候補を更新
fn update_suggestions(app: &mut App) {
    // InputTrackDataのフォーム表示中はサジェスト不要
    if matches!(app.screen, Screen::InputTrackData) && !app.form_fields.is_empty() {
        app.suggestions.clear();
        return;
    }

    let input = if !app.form_fields.is_empty() {
        app.form_fields[app.form_index].value.clone()
    } else {
        app.input_buffer.clone()
    };

    if input.is_empty() {
        // 全候補を表示
        match &app.screen {
            Screen::InputTrackData | Screen::SearchTrack => {
                if app.current_artist.is_empty() {
                    app.suggestions = app.db.get_all_artists().unwrap_or_default();
                } else {
                    app.suggestions = app
                        .db
                        .get_tracks_by_artist(&app.current_artist)
                        .unwrap_or_default();
                }
            }
            Screen::SearchWriter => {
                app.suggestions = app.db.get_writer_suggestions("").unwrap_or_default();
            }
            Screen::InputCreditData => {
                // Artistフィールド（form_index == 0）でArtistData候補
                if app.form_index == 0 {
                    app.suggestions = app.db.get_artist_data_names().unwrap_or_default();
                } else {
                    app.suggestions.clear();
                }
            }
            Screen::InputArtistData => {
                if app.form_index == 0 {
                    app.suggestions = app.db.get_artists_without_label().unwrap_or_default();
                } else if app.form_index == 1 {
                    app.suggestions = app.db.get_all_labels().unwrap_or_default();
                } else {
                    app.suggestions.clear();
                }
            }
            Screen::InputWriterData => {
                // Nameフィールド（form_index == 0）でライター名候補
                if app.form_index == 0 {
                    app.suggestions = app.db.get_credit_names().unwrap_or_default();
                } else {
                    app.suggestions.clear();
                }
            }
            _ => {}
        }
        app.suggestion_index = 0;
        return;
    }

    // 入力に基づいて候補をフィルタ（先頭一致）
    let input_lower = input.to_lowercase();
    match &app.screen {
        Screen::InputTrackData | Screen::SearchTrack => {
            if app.current_artist.is_empty() {
                // アーティスト候補
                let all_artists = app.db.get_all_artists().unwrap_or_default();
                app.suggestions = all_artists
                    .into_iter()
                    .filter(|a| a.to_lowercase().starts_with(&input_lower))
                    .collect();
            } else {
                // 曲候補
                let all_tracks = app
                    .db
                    .get_tracks_by_artist(&app.current_artist)
                    .unwrap_or_default();
                app.suggestions = all_tracks
                    .into_iter()
                    .filter(|t| t.to_lowercase().starts_with(&input_lower))
                    .collect();
            }
        }
        Screen::SearchWriter => {
            app.suggestions = app.db.get_writer_suggestions(&input).unwrap_or_default();
        }
        Screen::InputArtistData => {
            if app.form_index == 0 {
                let all_artists = app.db.get_artists_without_label().unwrap_or_default();
                app.suggestions = all_artists
                    .into_iter()
                    .filter(|a| a.to_lowercase().starts_with(&input_lower))
                    .collect();
            } else if app.form_index == 1 {
                let all_labels = app.db.get_all_labels().unwrap_or_default();
                app.suggestions = all_labels
                    .into_iter()
                    .filter(|l| l.to_lowercase().starts_with(&input_lower))
                    .collect();
            } else {
                app.suggestions.clear();
            }
        }
        Screen::InputCreditData => {
            // Artistフィールド（form_index == 0）でArtistData補完
            if app.form_index == 0 {
                let all_artists = app.db.get_artist_data_names().unwrap_or_default();
                app.suggestions = all_artists
                    .into_iter()
                    .filter(|a| a.to_lowercase().starts_with(&input_lower))
                    .collect();
            } else {
                app.suggestions.clear();
            }
        }
        Screen::InputWriterData => {
            // Nameフィールド（form_index == 0）でライター名補完
            if app.form_index == 0 {
                let all_writers = app.db.get_credit_names().unwrap_or_default();
                app.suggestions = all_writers
                    .into_iter()
                    .filter(|w| w.to_lowercase().starts_with(&input_lower))
                    .collect();
            } else {
                app.suggestions.clear();
            }
        }
        _ => {}
    }
    app.suggestion_index = 0;
}

/// スペースキー（Best16トグル等）
fn handle_space(app: &mut App) {
    match &app.screen {
        Screen::ViewTrackData => {
            if let Some(item) = app.tracks.get(app.list_index) {
                let artist = item.artist.clone();
                let track = item.track.clone();
                match app.db.toggle_soty(&artist, &track) {
                    Ok(is_best) => {
                        let status = if is_best { "ON" } else { "OFF" };
                        app.show_message(&format!("SOTY {} for '{}'", status, track));
                        reload_tracks(app);
                    }
                    Err(e) => {
                        app.show_error(&format!("Failed: {}", e));
                    }
                }
            }
        }
        _ => {}
    }
}

/// URLを開く
fn handle_open_url(_app: &mut App) {
}

/// コマンドファイルのパス
const SPOTIFY_CMD_FILE: &str = "/tmp/kpop4-spotify-cmd";
const SPOTIFY_STATUS_FILE: &str = "/tmp/kpop4-spotify-status";

/// Spotify再生を開始
fn handle_open_spotify(app: &mut App) {
    let url = if matches!(app.screen, Screen::InputTrackData) {
        // InputTrackData: フォームのSpotify URLフィールドから取得
        if app.form_fields.len() > 2 {
            let val = app.form_fields[2].value.trim().to_string();
            if val.is_empty() { None } else { Some(val) }
        } else {
            None
        }
    } else if matches!(app.screen, Screen::ViewTrackData) {
        // View画面: カーソル位置のトラックからSpotify URLを取得
        app.tracks.get(app.list_index)
            .and_then(|t| t.spotify.clone())
            .filter(|s| !s.is_empty())
    } else if matches!(app.screen, Screen::SearchTrackResult { .. } | Screen::MainMenu) {
        // SearchTrackResult / MainMenu: search_track_dataからSpotify URLを取得
        app.search_track_data.as_ref()
            .and_then(|td| td.spotify.clone())
            .filter(|s| !s.is_empty())
    } else if matches!(app.screen, Screen::SearchWriterResult { .. }) {
        // SearchWriterResult: カーソル位置の曲のSpotify URLをtrack_dataから取得
        app.search_results.get(app.list_index)
            .and_then(|c| app.db.get_song_add(&c.artist, &c.track).ok().flatten())
            .and_then(|td| td.spotify)
            .filter(|s| !s.is_empty())
    } else {
        None
    };

    let Some(url) = url else {
        app.show_error("No Spotify URL");
        return;
    };

    // 同じURLで一時停止中なら再開
    if url == app.spotify_url && !app.spotify_playing && spotify_process_alive() {
        send_spotify_command("toggle");
        app.spotify_playing = true;
        app.show_message("Resumed");
        return;
    }

    // URLが変わった or 新規 → プロセス再起動
    stop_spotify(app);
    std::thread::sleep(std::time::Duration::from_millis(500));
    let _ = std::fs::remove_file(SPOTIFY_CMD_FILE);
    let _ = std::fs::remove_file(SPOTIFY_STATUS_FILE);

    match std::process::Command::new("xvfb-run")
        .args(["--auto-servernum", "npx", "tsx", "spotify-play.ts", &url])
        .current_dir(dirs::home_dir().unwrap_or_default().join("ssbrowse"))
        .env("PULSE_SERVER", "/run/user/1000/pulse/native")
        .env("XDG_RUNTIME_DIR", "/run/user/1000")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => {
            app.spotify_url = url.clone();

            let (tx, rx) = std::sync::mpsc::channel();
            app.spotify_receiver = Some(rx);

            std::thread::spawn(move || {
                // ステータスファイルをポーリング（100ms間隔、最大60秒）
                for _ in 0..600 {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    if let Ok(status) = std::fs::read_to_string(SPOTIFY_STATUS_FILE) {
                        let status = status.trim().to_string();
                        if !status.is_empty() {
                            let _ = std::fs::remove_file(SPOTIFY_STATUS_FILE);
                            if status.starts_with("Playing") || status.starts_with("Already playing") {
                                let _ = tx.send(Ok(status));
                            } else {
                                let _ = tx.send(Err(status));
                            }
                            return;
                        }
                    }
                }
                let _ = tx.send(Err("Spotify timeout".to_string()));
            });
        }
        Err(e) => app.show_error(&format!("Failed to play: {}", e)),
    }
}

/// Spotifyプロセスを終了（アプリ終了時用）
fn stop_spotify(app: &mut App) {
    let _ = std::process::Command::new("pkill")
        .args(["-f", "spotify-play"])
        .output();
    let _ = std::process::Command::new("pkill")
        .args(["-f", "chrome.*chrome-data[^-]"])
        .output();
    app.spotify_playing = false;
}

/// spotify-playプロセスが生きているか
fn spotify_process_alive() -> bool {
    std::process::Command::new("pgrep")
        .args(["-f", "spotify-play"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Spotifyにコマンドを送信
fn send_spotify_command(cmd: &str) {
    let _ = std::fs::write(SPOTIFY_CMD_FILE, cmd);
}

/// 文字インデックスをバイトインデックスに変換
fn char_to_byte_index(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(byte_idx, _)| byte_idx)
        .unwrap_or(s.len())
}

/// 文字インデックスの位置の文字を削除
fn remove_char_at(s: &mut String, char_idx: usize) {
    let mut chars: Vec<char> = s.chars().collect();
    if char_idx < chars.len() {
        chars.remove(char_idx);
        *s = chars.into_iter().collect();
    }
}

/// バックグラウンドでsongbpm.comスクレイプを開始
fn start_bpm_scrape(app: &mut App) {
    let artist = app.current_artist.clone();

    // 同じアーティストのキャッシュがあればスキップ
    if app.bpm_cache_artist == artist && app.bpm_cache.is_some() {
        return;
    }

    let url = make_songbpm_url(&artist);
    let (tx, rx) = mpsc::channel();
    app.bpm_receiver = Some(rx);
    app.bpm_cache_artist = artist.clone();
    app.bpm_cache = None;
    app.loading = true;
    app.loading_message = format!("Fetching BPM: {}", artist);

    std::thread::spawn(move || {
        let result = scrape_songbpm(&url);
        let _ = tx.send(result.map_err(|e| e.to_string()));
    });
}

/// TrackDataフォームをセットアップ
fn setup_track_form(app: &mut App, dur: &str, bpm: Option<String>, spotify: &str, bpm_track_name: &str) {
    let bpm_field = if let Some(ref b) = bpm {
        if let Ok(n) = b.parse::<i64>() {
            let half = n / 2;
            let double = n * 2;
            FormField::select(
                "BPM",
                vec![
                    &half.to_string(),
                    b,
                    &double.to_string(),
                    "MIXX",
                ],
                1,
            )
        } else {
            FormField::with_value("BPM", b)
        }
    } else {
        FormField::new("BPM")
    };

    // Track名はフォーム上部の表示専用ヘッダー
    app.form_header = bpm_track_name.to_string();

    app.form_fields = vec![
        FormField::with_value("Duration (sec)", dur),
        bpm_field,
        FormField::with_value("Spotify URL", spotify),
        FormField::select("Release", vec!["-", "Title", "Pre"], 0),
    ];
    app.form_index = 0;
    app.mode = Mode::Normal;
    app.suggestions.clear();
}

/// BPMキャッシュからマッチを検索し、1件なら即フォーム、複数なら選択UIを表示
fn handle_bpm_matches(app: &mut App) {
    let mut matches: Vec<BpmTrackInfo> = if let Some(ref cache) = app.bpm_cache {
        find_all_tracks_in_bpm_data(&cache.tracks, &app.current_track)
            .into_iter()
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    // 重複除去（track_name, bpm, duration が同じものを除く）
    matches.dedup_by(|a, b| {
        a.track_name == b.track_name && a.bpm == b.bpm && a.duration == b.duration
    });

    if matches.is_empty() {
        app.bpm_matches = Vec::new();
        app.bpm_pending_matches = Vec::new();
        app.suggestion_index = 0;
        fill_form_from_bpm_match(app);
    } else {
        // 最初の1件で即フォーム表示、残りはpendingへ
        let first = matches.remove(0);
        app.bpm_matches = vec![first];
        app.bpm_pending_matches = matches;
        app.suggestion_index = 0;
        fill_form_from_bpm_match(app);
    }
}

/// pendingマッチを1件ずつbpm_matchesに移動（main loopから毎tick呼ばれる）
pub fn tick_bpm_pending(app: &mut App) {
    if app.bpm_pending_matches.is_empty() {
        return;
    }
    let next = app.bpm_pending_matches.remove(0);
    app.bpm_matches.push(next);
}

/// bpm_matches[suggestion_index]でフォームを埋める
fn fill_form_from_bpm_match(app: &mut App) {
    if let Some(m) = app.bpm_matches.get(app.suggestion_index) {
        let dur = m.duration.clone().unwrap_or_default();
        let spotify = m.spotify_url.clone().unwrap_or_default();
        let track_name = m.track_name.clone();
        setup_track_form(app, &dur, m.bpm.clone(), &spotify, &track_name);
    } else {
        setup_track_form(app, "", None, "", "");
    }
}

/// BPMスクレイピング結果を処理
pub fn process_bpm_result(app: &mut App, result: Result<BpmArtistInfo, String>) {
    match result {
        Ok(info) => {
            app.bpm_cache = Some(info);

            // スクレイプ完了 → マッチ処理
            if matches!(app.screen, Screen::InputTrackData)
                && app.form_fields.is_empty()
                && !app.current_track.is_empty()
            {
                handle_bpm_matches(app);
            }
        }
        Err(_) => {
            // スクレイプ失敗 → 空フォームを表示して手入力可
            if matches!(app.screen, Screen::InputTrackData)
                && app.form_fields.is_empty()
                && !app.current_track.is_empty()
            {
                setup_track_form(app, "", None, "", "");
            }
        }
    }
}

/// WriterAka: Akaトグル（Spaceで呼ばれる）
fn handle_writer_aka_toggle(app: &mut App) {
    if let Some((name_a, name_b, _is_aka)) = app.aka_pairs.get(app.list_index).cloned() {
        match app.db.toggle_writer_aka(&name_a, &name_b) {
            Ok(new_state) => {
                app.aka_pairs[app.list_index].2 = new_state;
                let status = if new_state { "Aka ON" } else { "Aka OFF" };
                app.show_message(&format!("{} ↔ {} {}", name_a, name_b, status));
            }
            Err(e) => app.show_error(&format!("Failed: {}", e)),
        }
    }
}

/// スクレイピング結果を処理
pub fn process_scrape_result(app: &mut App, result: Result<ScrapedSongInfo, String>) {
    match result {
        Ok(info) => {
            // アーティストデータを確認
            let artist_not_found = app.db.get_artist(&info.artist).ok().flatten().is_none();

            // クレジットごとにCreditDataを挿入
            let mut inserted = 0;
            for credit in &info.credits {
                let song = CreditData {
                    id: None,
                    artist: info.artist.clone(),
                    label: None,
                    date: info.date.clone(),
                    album: info.album.clone(),
                    track: info.track.clone(),
                    role: Some(credit.role.clone()),
                    name: Some(credit.name.clone()),
                    count: None,
                    created_at: None,
                    is_aoty: false,
                    is_soty: false,
                };

                if app.db.insert_song(&song).is_ok() {
                    inserted += 1;
                    let _ = app.db.update_writer_count(&credit.name);
                }
            }

            // アーティストがDBにない場合、ArtistData画面に遷移
            if artist_not_found {
                let artist_name = info.artist.clone();
                app.go_to(Screen::InputArtistData);
                // go_toでform_fieldsが初期化された後にアーティスト名を自動入力
                if !app.form_fields.is_empty() {
                    app.form_fields[0].value = artist_name.clone();
                    app.form_fields[0].cursor = artist_name.chars().count();
                }
                // Labelフィールドにカーソルを置いてInsertモード
                app.form_index = 1;
                app.mode = Mode::Insert;
                app.suggestions.clear();
                // go_toがmessageをクリアするので、遷移後にメッセージをセット
                app.show_message(&format!(
                    "Added {} credits for '{}'. New artist '{}' - please register.",
                    inserted, info.track, artist_name
                ));
                return;
            }

            if inserted > 0 {
                app.show_message(&format!(
                    "Added {} credits for '{}'",
                    inserted, info.track
                ));
            } else {
                app.show_message("No new credits found");
            }

            // Trackフィールドのみクリア、Artistはそのまま
            if app.form_fields.len() >= 2 {
                app.form_fields[1].value.clear();
                app.form_fields[1].cursor = 0;
            }
            app.form_index = 1;
            app.mode = Mode::Normal;
        }
        Err(e) => {
            app.show_error(&format!("Scraping failed: {}", e));
        }
    }
}
