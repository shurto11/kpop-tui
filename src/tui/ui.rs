use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, Wrap, BarChart, Bar, BarGroup},
    Frame,
};
use unicode_width::UnicodeWidthChar;

use chrono::{Datelike, Local, NaiveDate};
use crate::tui::app::{App, AutoAddPhase, AutoAddRow, AutoAddStatus, Mode, Screen};

/// AOTY/SOTY用の金色
const GOLD: Color = Color::Rgb(255, 215, 0);

/// 誕生日文字列から年齢を計算
fn calc_age(birth: &str) -> Option<i32> {
    // "YYYY-M-D" or "YYYY-MM-DD"
    let parts: Vec<&str> = birth.split('-').collect();
    if parts.len() != 3 { return None; }
    let y: i32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    let d: u32 = parts[2].parse().ok()?;
    let bd = NaiveDate::from_ymd_opt(y, m, d)?;
    let today = Local::now().date_naive();
    let mut age = today.year() - bd.year();
    if (today.month(), today.day()) < (bd.month(), bd.day()) {
        age -= 1;
    }
    Some(age)
}

/// 文字列の先頭からchar_count文字分の表示幅を計算
fn display_width_up_to(s: &str, char_count: usize) -> u16 {
    s.chars()
        .take(char_count)
        .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
        .sum::<usize>() as u16
}

/// ロール名に対応する色を返す
fn role_color(role: &str) -> Color {
    match role.to_lowercase().as_str() {
        "lyricist" => Color::Rgb(100, 200, 130),  // 落ち着いた緑
        "composer" => Color::Rgb(210, 180, 80),    // 柔らかい黄
        "arranger" => Color::Rgb(200, 100, 100),   // 落ち着いた赤
        "writer"   => Color::Rgb(170, 120, 200),   // 落ち着いた紫
        _ => Color::Reset,
    }
}

/// 行スタイルを決定（カーソル行 + 検索ハイライト）
fn row_style(app: &App, actual_index: usize) -> Style {
    let is_cursor = actual_index == app.list_index;
    let is_match = !app.search_query.is_empty() && app.search_match_indices.contains(&actual_index);

    if is_cursor && is_match {
        Style::default().fg(Color::Yellow).bg(Color::DarkGray).add_modifier(Modifier::BOLD)
    } else if is_cursor {
        Style::default().fg(Color::Yellow)
    } else if is_match {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    }
}

/// メイン描画関数
pub fn draw(frame: &mut Frame, app: &mut App) {
    let is_searching = app.mode == Mode::Search;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if is_searching {
            vec![
                Constraint::Length(3), // ヘッダー
                Constraint::Min(0),    // メイン
                Constraint::Length(3), // 検索入力欄
                Constraint::Length(3), // フッター
            ]
        } else {
            vec![
                Constraint::Length(3), // ヘッダー
                Constraint::Min(0),    // メイン
                Constraint::Length(3), // フッター
            ]
        })
        .split(frame.area());

    // visible_rowsを更新（メインエリアの高さ - ボーダー2行 - ヘッダー1行）
    let main_height = chunks[1].height as usize;
    if main_height > 4 {
        app.visible_rows = main_height - 4;
    }

    // カーソルが見える範囲にlist_offsetを補正
    if app.list_index >= app.list_offset + app.visible_rows {
        app.list_offset = app.list_index - app.visible_rows + 1;
    }
    if app.list_index < app.list_offset {
        app.list_offset = app.list_index;
    }

    draw_header(frame, app, chunks[0]);
    draw_main(frame, app, chunks[1]);

    if is_searching {
        // 検索入力欄
        let search_input = Paragraph::new(Line::from(vec![
            Span::styled("/", Style::default().fg(Color::Yellow)),
            Span::raw(&app.input_buffer),
        ]))
        .block(Block::default().borders(Borders::ALL).title("Search"));

        frame.render_widget(search_input, chunks[2]);

        // カーソル表示
        let cursor_x = chunks[2].x + 1 + 1 + display_width_up_to(&app.input_buffer, app.input_cursor);
        let cursor_y = chunks[2].y + 1;
        frame.set_cursor_position((cursor_x, cursor_y));

        draw_footer(frame, app, chunks[3]);
    } else {
        draw_footer(frame, app, chunks[2]);
    }
}

/// ヘッダー
fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let title = match &app.screen {
        Screen::MainMenu => "kpop-tui",
        Screen::InputMenu => "Input",
        Screen::InputAutoAdd => "Input > AutoAdd",
        Screen::InputCreditData => "Input > CreditData",
        Screen::InputTrackData => "Input > TrackData",
        Screen::InputArtistData => "Input > ArtistData",
        Screen::InputWriterData => "Input > WriterData",
        Screen::InputWriterAka => "Input > WriterAka",
        Screen::SearchMenu => "Search",
        Screen::SearchWriter => "Search > Writer",
        Screen::SearchWriterResult { name } => {
            return draw_header_with_name(frame, "Search > Writer", name, area);
        }
        Screen::SearchTrack => "Search > Track",
        Screen::SearchTrackResult { artist, track } => {
            return draw_header_with_song(frame, app, artist, track, area);
        }
        Screen::ViewMenu => "View",
        Screen::ViewLog => "View > Log",
        Screen::ViewCreditData => "View > CreditData",
        Screen::ViewTrackData => "View > TrackData",
        Screen::ViewArtistData => "View > ArtistData",
        Screen::ViewWriterData => "View > WriterData",
        Screen::Quiz => {
            let q = app.quiz_current + 1;
            let total = app.quiz_questions.len();
            let score = app.quiz_score;
            return draw_quiz_header(frame, q, total, score, app.mode, area);
        }
        Screen::QuizResult => {
            let q = app.quiz_current + 1;
            let total = app.quiz_questions.len();
            let score = app.quiz_score;
            return draw_quiz_header(frame, q, total, score, app.mode, area);
        }
        Screen::QuizFinal => "Quiz > Result",
    };

    let mode_str = match app.mode {
        Mode::Normal => "[NORMAL]",
        Mode::Insert => "[INSERT]",
        Mode::Search => "[SEARCH]",
        Mode::Visual => "[VISUAL]",
    };

    let header = Paragraph::new(Line::from(vec![
        Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(mode_str, Style::default().fg(Color::Yellow)),
    ]))
    .block(Block::default().borders(Borders::ALL));

    frame.render_widget(header, area);
}

fn draw_header_with_name(frame: &mut Frame, prefix: &str, name: &str, area: Rect) {
    let header = Paragraph::new(Line::from(vec![
        Span::styled(prefix, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(": "),
        Span::styled(name, Style::default().fg(Color::Cyan)),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(header, area);
}

fn draw_header_with_song(frame: &mut Frame, app: &App, artist: &str, track: &str, area: Rect) {
    let is_soty = app.search_track_data.as_ref().map_or(false, |td| td.is_soty);
    let track_color = if is_soty { GOLD } else { Color::Green };
    let header = Paragraph::new(Line::from(vec![
        Span::styled("Search > Track", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(": "),
        Span::styled(artist, Style::default().fg(Color::Cyan)),
        Span::raw(" - "),
        Span::styled(track, Style::default().fg(track_color)),
    ]))
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(header, area);
}

/// メインエリア
fn draw_main(frame: &mut Frame, app: &mut App, area: Rect) {
    match &app.screen {
        Screen::MainMenu | Screen::InputMenu | Screen::SearchMenu | Screen::ViewMenu => {
            draw_main_menu(frame, app, area);
        }
        Screen::InputCreditData | Screen::InputArtistData | Screen::InputWriterData => {
            draw_form(frame, app, area);
        }
        Screen::InputWriterAka => {
            draw_writer_aka(frame, app, area);
        }
        Screen::InputAutoAdd => {
            draw_auto_add(frame, app, area);
        }
        Screen::InputTrackData => {
            // フォーム表示中はフォーム描画、それ以外はSuggestions選択
            if !app.form_fields.is_empty() {
                draw_song_add_form(frame, app, area);
            } else {
                draw_suggestions_only(frame, app, area);
            }
        }
        Screen::SearchTrack => {
            if !app.current_artist.is_empty() {
                // Track選択: 入力欄なし、Suggestionsのみ
                draw_suggestions_with_title(frame, app, area, &format!("{} > Track", app.current_artist));
            } else {
                draw_input_with_suggestions(frame, app, area);
            }
        }
        Screen::SearchWriter => {
            draw_simple_input(frame, app, area);
        }
        Screen::ViewLog | Screen::ViewCreditData => {
            draw_song_table(frame, app, area);
        }
        Screen::ViewArtistData => {
            draw_artist_table(frame, app, area);
        }
        Screen::ViewWriterData => {
            draw_writer_table(frame, app, area);
        }
        Screen::ViewTrackData => {
            draw_track_table(frame, app, area);
        }
        Screen::SearchWriterResult { .. } => {
            draw_writer_result(frame, app, area);
        }
        Screen::SearchTrackResult { .. } => {
            draw_song_result(frame, app, area);
        }
        Screen::Quiz => {
            if !app.current_artist.is_empty() {
                draw_suggestions_with_title(frame, app, area, &format!("{} > Track", app.current_artist));
            } else {
                draw_input_with_suggestions(frame, app, area);
            }
        }
        Screen::QuizResult => {
            draw_quiz_result(frame, app, area);
        }
        Screen::QuizFinal => {
            draw_quiz_final(frame, app, area);
        }
    }
}

/// フッター（キーヒント / ローディング / メッセージ / エラー）
fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    // ローディング中（Spotify以外）
    if app.loading {
        let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let spinner_char = spinner[app.tick % spinner.len()];
        let text = format!("{} {}  [Esc] Cancel", spinner_char, app.loading_message);
        let footer = Paragraph::new(text)
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL));
        frame.render_widget(footer, area);
        return;
    }

    // エラー表示
    if let Some(ref err) = app.error {
        let footer = Paragraph::new(err.as_str())
            .style(Style::default().fg(Color::Red))
            .block(Block::default().borders(Borders::ALL).title("Error"));
        frame.render_widget(footer, area);
        return;
    }

    // メッセージ表示
    if let Some(ref msg) = app.message {
        let footer = Paragraph::new(msg.as_str())
            .style(Style::default().fg(Color::Green))
            .block(Block::default().borders(Borders::ALL).title("Message"));
        frame.render_widget(footer, area);
        return;
    }

    // 通常のキーヒント
    let spotify_indicator = if app.spotify_receiver.is_some() {
        let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let ch = spinner[app.tick % spinner.len()];
        format!("  {} Spotify", ch)
    } else {
        String::new()
    };

    let hints = match app.mode {
        Mode::Normal => {
            match &app.screen {
                Screen::MainMenu => {
                    "j/k: Move  l/Enter: Select  c: Play  x: Pause  Q: Quit"
                }
                Screen::InputMenu | Screen::SearchMenu | Screen::ViewMenu => {
                    "j/k: Move  l/Enter: Select  h/Esc: Back  q: Menu  Q: Quit"
                }
                Screen::InputCreditData | Screen::InputArtistData
                | Screen::InputWriterData | Screen::SearchWriter | Screen::SearchTrack => {
                    "i: Insert  h/Esc: Back  q: Menu  Q: Quit"
                }
                Screen::InputTrackData => {
                    "l: Select  a: Next BPM  i: Insert  h/Esc: Back  q: Menu  Q: Quit"
                }
                Screen::InputWriterAka => {
                    "j/k: Move  Space: Toggle  h/Esc: Back  q: Menu  Q: Quit"
                }
                Screen::InputAutoAdd => {
                    if app.auto_add_editing.is_some() {
                        "Enter: Save  Tab: Next Field  Esc: Cancel  ←→: Cursor  BS: Delete"
                    } else {
                        match app.auto_add_phase {
                            AutoAddPhase::Fetching => "Esc: Cancel",
                            AutoAddPhase::Checking => "Esc: Cancel scan",
                            _ => "j/k: Move  e: Edit  d: Delete  r: Recheck  o: Open  Enter: Add All  h/Esc: Back",
                        }
                    }
                }
                Screen::ViewLog => {
                    if app.editing_log_album {
                        "Enter: Save  Esc: Cancel  ←→: Cursor  BS: Delete"
                    } else {
                        "j/k: Move  e: Edit Album  d: Delete Song  r: Rewind  l: Writer  /: Search  c: Play  h: Back"
                    }
                }
                Screen::ViewCreditData => {
                    "j/k: Move  l: Writer  /: Search  h: Back  q: Menu  Q: Quit"
                }
                Screen::ViewTrackData => {
                    "j/k: Move  l: Detail  e: Edit  s: SOTY  a: AOTY  S: Toggle SOTY  A: Toggle AOTY  c: Play  /: Search  h: Back"
                }
                Screen::ViewArtistData => {
                    "j/k: Move  v: Visual  e: Edit  u: Undo  Ctrl+r: Redo  /: Search  h: Back  q: Menu"
                }
                Screen::ViewWriterData => {
                    "j/k: Move  l: Songs  e: Edit  /: Search  h: Back  q: Menu  Q: Quit"
                }
                Screen::SearchWriterResult { .. } => {
                    "j/k: Move  l: Detail  e: Edit Writer  c: Play  /: Search  h: Back  q: Menu  Q: Quit"
                }
                Screen::SearchTrackResult { .. } => {
                    "j/k: Move  l: Writer  c: Play  x: Pause  /: Search  h: Back  q: Menu  Q: Quit"
                }
                Screen::Quiz => {
                    "i: Insert  Tab: Suggestions  p: Pass  c: Play  h/Esc: Back  q: Menu"
                }
                Screen::QuizResult => {
                    "Enter/l: Next  Esc: Next  q: Menu"
                }
                Screen::QuizFinal => {
                    "Enter/Esc: Menu"
                }
            }
        }
        Mode::Insert => {
            if matches!(app.screen, Screen::ViewArtistData) && app.editing_field.is_some() {
                "Tab/↑↓: Switch field  jj: Save & Exit  Esc: Cancel"
            } else {
                "Esc: Normal  Tab: Suggestions  Enter: Confirm  Ctrl+C: Cancel"
            }
        }
        Mode::Search => "Esc: Cancel  Enter: Search  n/N: Next/Prev",
        Mode::Visual => "j/k: Move item  d: Delete  u: Undo  Ctrl+r: Redo  Esc/v: Exit",
    };

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(hints, Style::default().fg(Color::DarkGray)),
        Span::styled(&spotify_indicator, Style::default().fg(Color::Yellow)),
    ]))
    .block(Block::default().borders(Borders::ALL));

    frame.render_widget(footer, area);
}

/// メニュー描画
fn draw_menu(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .menu_items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let style = if i == app.menu_index {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let prefix = if i == app.menu_index { "> " } else { "  " };
            ListItem::new(format!("{}{}", prefix, item.label)).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Menu"));

    frame.render_widget(list, area);
}

/// MainMenu描画（ランダム曲情報 + メニュー）
fn draw_main_menu(frame: &mut Frame, app: &mut App, area: Rect) {
    // ランダム曲データがあるかチェック
    let has_track = app.search_track_data.is_some() || !app.search_results.is_empty();

    if !has_track {
        // データがない場合は通常メニューのみ
        draw_menu(frame, app, area);
        return;
    }

    // 曲名バー(高さ3: ボーダー2+テキスト1) + Track情報(高さ13) + メニュー(残り)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),   // Random Track タイトルバー
            Constraint::Length(13),  // Track情報(Art + TrackData + Around-The-Day)
            Constraint::Min(0),     // メニュー
        ])
        .split(area);

    // Random Track タイトルバー
    let (artist, track) = if let Some(ref td) = app.search_track_data {
        (td.artist.clone(), td.track.clone())
    } else if let Some(credit) = app.search_results.first() {
        (credit.artist.clone(), credit.track.clone())
    } else {
        (String::new(), String::new())
    };

    let is_soty = app.search_track_data.as_ref().map_or(false, |td| td.is_soty);
    let track_color = if is_soty { GOLD } else { Color::Green };
    let label = if app.is_random_fallback { "New Track: " } else { "Today's Drops: " };
    let title_bar = Paragraph::new(Line::from(vec![
        Span::styled(label, Style::default().fg(Color::DarkGray)),
        Span::styled(&artist, Style::default().fg(Color::Cyan)),
        Span::raw(" - "),
        Span::styled(&track, Style::default().fg(track_color)),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title_bar, chunks[0]);

    // Track情報エリア: Art | TrackData | Around-The-Day Drops
    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(22), // Album: 20文字 + ボーダー2
            Constraint::Fill(1),    // TrackData (均等)
            Constraint::Fill(1),    // Around-The-Day Drops (均等)
        ])
        .split(chunks[1]);

    // Album Art
    if let Some(ref art_lines) = app.album_art_current {
        let art_text: Vec<Line> = art_lines.iter().map(|row| {
            let spans: Vec<Span> = row.iter().map(|&(ch, r, g, b)| {
                Span::styled(
                    ch.to_string(),
                    Style::default().fg(Color::Indexed(rgb_to_256(r, g, b))),
                )
            }).collect();
            Line::from(spans)
        }).collect();
        let art = Paragraph::new(art_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Art")
                    .border_style(Style::default().fg(Color::DarkGray)),
            );
        frame.render_widget(art, top_chunks[0]);
    } else {
        draw_art_placeholder(frame, app, top_chunks[0]);
    }

    // TrackData
    if let Some(ref td) = app.search_track_data {
        draw_track_data_vertical(frame, td, app.search_results.first(), app.search_artist_label.as_deref(), top_chunks[1]);
    } else {
        let empty = Block::default()
            .borders(Borders::ALL)
            .title("TrackData")
            .border_style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty, top_chunks[1]);
    }

    // Around-The-Day Drops
    draw_around_day(frame, app, top_chunks[2], "Around-The-Day Drops");

    // メニュー
    draw_menu(frame, app, chunks[2]);
}

/// フォーム描画
fn draw_form(frame: &mut Frame, app: &App, area: Rect) {
    let has_header = !app.form_header.is_empty();
    let header_len = if has_header { 1 } else { 0 };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            std::iter::repeat(Constraint::Length(3))
                .take(header_len)
                .chain(app.form_fields.iter().map(|_| Constraint::Length(3)))
                .chain(std::iter::once(Constraint::Min(0)))
                .collect::<Vec<_>>(),
        )
        .split(area);

    // ヘッダー（表示専用、選択不可）
    if has_header {
        let header = Paragraph::new(app.form_header.as_str())
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Track")
                    .border_style(Style::default().fg(Color::DarkGray)),
            );
        frame.render_widget(header, chunks[0]);
    }

    for (i, field) in app.form_fields.iter().enumerate() {
        let is_selected = i == app.form_index;
        let is_editing = is_selected && app.mode == Mode::Insert;

        // ノーマルモードでも選択中フィールドをハイライト
        let style = if is_selected {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };

        let border_style = if is_editing {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else if is_selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };

        let cursor_pos = if is_editing { display_width_up_to(&field.value, field.cursor) } else { 0 };
        let display_value = if is_editing && field.value.is_empty() {
            "_".to_string()
        } else {
            field.value.clone()
        };

        // ノーマルモードで選択中は > を表示
        let title = if is_selected && app.mode == Mode::Normal {
            format!("> {}", field.label)
        } else {
            field.label.clone()
        };

        let input = Paragraph::new(display_value)
            .style(style)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(border_style),
            );

        let ci = i + header_len;
        frame.render_widget(input, chunks[ci]);

        // カーソル表示
        if is_editing {
            frame.set_cursor_position((
                chunks[ci].x + 1 + cursor_pos as u16,
                chunks[ci].y + 1,
            ));
        }
    }

    // 補完候補を表示（インサートモードまたはSuggestions選択中、かつサジェスチョン対象フィールド）
    if (app.mode == Mode::Insert || app.selecting_suggestion) && should_show_suggestions(app) {
        let remaining_area = chunks.last().copied().unwrap_or(area);
        draw_suggestions(frame, app, remaining_area);
    }
}

/// シンプル入力（SearchWriter）
fn draw_simple_input(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let style = if app.mode == Mode::Insert {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let display = if app.input_buffer.is_empty() && app.mode == Mode::Insert {
        "_".to_string()
    } else {
        app.input_buffer.clone()
    };

    let input = Paragraph::new(display).style(style).block(
        Block::default()
            .borders(Borders::ALL)
            .title(app.input_label.as_str())
            .border_style(if app.mode == Mode::Insert {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            }),
    );

    frame.render_widget(input, chunks[0]);

    if app.mode == Mode::Insert {
        frame.set_cursor_position((
            chunks[0].x + 1 + display_width_up_to(&app.input_buffer, app.input_cursor),
            chunks[0].y + 1,
        ));
    }

    // 補完候補
    if !app.suggestions.is_empty() {
        draw_suggestions(frame, app, chunks[1]);
    }
}

/// 入力＋補完候補
fn draw_input_with_suggestions(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    // 入力欄
    let label = if app.current_artist.is_empty() {
        &app.input_label
    } else {
        &format!("{} > Track", app.current_artist)
    };

    let display = if app.input_buffer.is_empty() && app.mode == Mode::Insert {
        "_".to_string()
    } else {
        app.input_buffer.clone()
    };

    let style = if app.mode == Mode::Insert {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let input = Paragraph::new(display).style(style).block(
        Block::default()
            .borders(Borders::ALL)
            .title(label.as_str())
            .border_style(if app.mode == Mode::Insert {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            }),
    );

    frame.render_widget(input, chunks[0]);

    if app.mode == Mode::Insert {
        frame.set_cursor_position((
            chunks[0].x + 1 + display_width_up_to(&app.input_buffer, app.input_cursor),
            chunks[0].y + 1,
        ));
    }

    // 補完候補
    draw_suggestions(frame, app, chunks[1]);
}

/// 補完候補リスト
fn draw_suggestions(frame: &mut Frame, app: &App, area: Rect) {
    // 表示可能行数（ボーダー2行分を引く）
    let visible = area.height.saturating_sub(2) as usize;
    let total = app.suggestions.len();
    let idx = app.suggestion_index;

    // スクロールオフセットを計算
    let offset = if visible == 0 || total <= visible {
        0
    } else if idx < visible / 2 {
        0
    } else if idx + visible / 2 >= total {
        total - visible
    } else {
        idx - visible / 2
    };
    let end = (offset + visible).min(total);

    let items: Vec<ListItem> = app
        .suggestions[offset..end]
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let actual = offset + i;
            let style = if actual == idx {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let prefix = if actual == idx { "> " } else { "  " };
            ListItem::new(format!("{}{}", prefix, s)).style(style)
        })
        .collect();

    // Suggestions選択中はボーダーをハイライト
    let (title, border_style) = if app.selecting_suggestion {
        (
            "Suggestions [j/k: move, l/Enter: select, h: cancel]",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )
    } else {
        ("Suggestions [Tab]", Style::default())
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(border_style),
    );

    frame.render_widget(list, area);
}

/// Suggestionsのみ表示（InputTrackData用）
fn draw_suggestions_only(frame: &mut Frame, app: &App, area: Rect) {
    let title = if app.current_artist.is_empty() {
        format!("Select Artist [j/k: move, l/Enter: select]")
    } else {
        format!("{} > Select Track [j/k: move, l/Enter: select, h: back]", app.current_artist)
    };

    // 表示可能行数（ボーダー2行分を引く）
    let visible = area.height.saturating_sub(2) as usize;
    let total = app.suggestions.len();
    let idx = app.suggestion_index;

    let offset = if visible == 0 || total <= visible {
        0
    } else if idx < visible / 2 {
        0
    } else if idx + visible / 2 >= total {
        total - visible
    } else {
        idx - visible / 2
    };
    let end = (offset + visible).min(total);

    let items: Vec<ListItem> = app
        .suggestions[offset..end]
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let actual = offset + i;
            let style = if actual == idx {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let prefix = if actual == idx { "> " } else { "  " };
            ListItem::new(format!("{}{}", prefix, s)).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Cyan)),
    );

    frame.render_widget(list, area);
}

/// タイトル付きSuggestions表示（SearchTrackのTrack選択用）
fn draw_suggestions_with_title(frame: &mut Frame, app: &App, area: Rect, title: &str) {
    let visible = area.height.saturating_sub(2) as usize;
    let total = app.suggestions.len();
    let idx = app.suggestion_index;

    let offset = if visible == 0 || total <= visible {
        0
    } else if idx < visible / 2 {
        0
    } else if idx + visible / 2 >= total {
        total - visible
    } else {
        idx - visible / 2
    };
    let end = (offset + visible).min(total);

    let items: Vec<ListItem> = app.suggestions[offset..end]
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let actual = offset + i;
            let style = if actual == idx {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let prefix = if actual == idx { "> " } else { "  " };
            ListItem::new(format!("{}{}", prefix, s)).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title.to_string())
            .border_style(Style::default().fg(Color::Cyan)),
    );

    frame.render_widget(list, area);
}

/// WriterAka画面描画（単語一致ペアの自動検出リスト）
fn draw_writer_aka(frame: &mut Frame, app: &App, area: Rect) {
    if app.aka_pairs.is_empty() {
        let empty = Paragraph::new("No word-matching pairs found")
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::ALL).title("Writer Aka"));
        frame.render_widget(empty, area);
        return;
    }

    let end = (app.list_offset + app.visible_rows).min(app.aka_pairs.len());
    let visible_pairs = &app.aka_pairs[app.list_offset..end];

    let items: Vec<ListItem> = visible_pairs
        .iter()
        .enumerate()
        .map(|(i, (name_a, name_b, is_aka))| {
            let actual_index = app.list_offset + i;
            let is_cursor = actual_index == app.list_index;
            let prefix = if is_cursor { "> " } else { "  " };
            let status = if *is_aka { "[Aka]" } else { "[ - ]" };
            let status_color = if *is_aka { Color::Green } else { Color::DarkGray };

            let line = Line::from(vec![
                Span::raw(prefix),
                Span::raw(name_a.as_str()),
                Span::styled(" ↔ ", Style::default().fg(Color::DarkGray)),
                Span::raw(name_b.as_str()),
                Span::raw("  "),
                Span::styled(status, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
            ]);

            let style = if is_cursor {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Writer Aka ({}) [Space: toggle, d: dismiss]", app.aka_pairs.len())),
    );

    frame.render_widget(list, area);
}

/// AutoAdd: Spotifyお気に入りからの追加候補一覧
fn draw_auto_add(frame: &mut Frame, app: &App, area: Rect) {
    let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let spin = spinner[app.tick % spinner.len()];

    if app.auto_add_rows.is_empty() {
        let (text, color) = match app.auto_add_phase {
            AutoAddPhase::Fetching => (
                format!("{} Fetching liked songs from Spotify...", spin),
                Color::Yellow,
            ),
            // design.md: 追加する曲がないことは英語で伝える
            AutoAddPhase::Empty => ("No songs to add".to_string(), Color::DarkGray),
            _ => ("No songs to add".to_string(), Color::DarkGray),
        };
        let empty = Paragraph::new(text)
            .style(Style::default().fg(color))
            .block(Block::default().borders(Borders::ALL).title("AutoAdd"));
        frame.render_widget(empty, area);
        return;
    }

    let end = (app.list_offset + app.visible_rows).min(app.auto_add_rows.len());
    let visible = &app.auto_add_rows[app.list_offset..end];

    let items: Vec<ListItem> = visible
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let actual_index = app.list_offset + i;
            let is_cursor = actual_index == app.list_index;
            let prefix = if is_cursor { "> " } else { "  " };

            let (mark, mark_color) = match row.status {
                AutoAddStatus::Pending => ("[ ]".to_string(), Color::DarkGray),
                AutoAddStatus::Checking => (format!("[{}]", spin), Color::Yellow),
                AutoAddStatus::Ok => ("[✓]".to_string(), Color::Green),
                AutoAddStatus::NotFound => ("[✗]".to_string(), Color::Red),
                AutoAddStatus::NetError => ("[!]".to_string(), Color::Yellow),
                AutoAddStatus::Duplicate => ("[dup]".to_string(), Color::DarkGray),
                AutoAddStatus::NoArtist => ("[✗]".to_string(), Color::Red),
            };

            let editing_track = is_cursor && app.auto_add_editing == Some(0);
            let editing_artist = is_cursor && app.auto_add_editing == Some(1);

            let mut spans = vec![
                Span::raw(prefix),
                Span::styled(
                    mark,
                    Style::default().fg(mark_color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
            ];

            if editing_track {
                spans.push(Span::styled(
                    format!(">{}", app.edit_buffer),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::raw(truncate_str(&row.track, 34)));
            }

            spans.push(Span::styled("  —  ", Style::default().fg(Color::DarkGray)));

            if editing_artist {
                spans.push(Span::styled(
                    format!(">{}", app.edit_buffer),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::raw(truncate_str(&row.artist, 24)));
            }

            // 追加日は YYYY-MM-DD だけ見せる
            let date = row.added_at.split('T').next().unwrap_or("").to_string();
            if !date.is_empty() {
                spans.push(Span::styled(
                    format!("  ({})", date),
                    Style::default().fg(Color::DarkGray),
                ));
            }

            // ✗ だった値は ✓ になった後も残す（何をどう直したか追えるように）
            if let Some(reason) = failed_note(row) {
                spans.push(Span::styled(
                    format!("   ✗ was: {}", reason),
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                ));
            }

            if row.status == AutoAddStatus::NoArtist {
                spans.push(Span::styled(
                    "  artist not registered (Enter to register)",
                    Style::default().fg(Color::Red),
                ));
            }

            ListItem::new(Line::from(spans)).style(row_style(app, actual_index))
        })
        .collect();

    let ok = app
        .auto_add_rows
        .iter()
        .filter(|r| r.status == AutoAddStatus::Ok)
        .count();
    let title = match app.auto_add_phase {
        AutoAddPhase::Checking => format!(
            "AutoAdd ({}) {} checking {}/{}",
            app.auto_add_rows.len(),
            spin,
            app.auto_add_done,
            app.auto_add_total
        ),
        _ => format!(
            "AutoAdd ({}) [{} ready] [e: edit, d: delete, r: recheck, Enter: add all]",
            app.auto_add_rows.len(),
            ok
        ),
    };

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(list, area);
}

/// ✗ だったときの値を1行で表す。編集で直した場合も自動で直った場合も残す。
/// 曲名・アーティスト名が変わっていない（＝URLの作り方だけが外れていた）場合は、
/// 外れたURLのスラッグを見せる
fn failed_note(row: &AutoAddRow) -> Option<String> {
    let track_changed = row.failed_track.as_ref().is_some_and(|t| *t != row.track);
    let artist_changed = row.failed_artist.as_ref().is_some_and(|a| *a != row.artist);

    match (track_changed, artist_changed) {
        (true, true) => Some(format!(
            "{} — {}",
            row.failed_track.as_ref().unwrap(),
            row.failed_artist.as_ref().unwrap()
        )),
        (true, false) => row.failed_track.clone(),
        (false, true) => row.failed_artist.clone(),
        // 名前は同じでURLだけ外れていた場合
        (false, false) => row.failed_url.as_ref().map(|u| {
            u.rsplit('/').next().unwrap_or(u.as_str()).to_string()
        }),
    }
}

/// TrackDataフォーム表示
fn draw_song_add_form(frame: &mut Frame, app: &App, area: Rect) {
    let has_header = !app.form_header.is_empty();
    let header_len = if has_header { 1 } else { 0 };

    let mut constraints: Vec<Constraint> = vec![Constraint::Length(2)]; // タイトル
    if has_header {
        constraints.push(Constraint::Length(3)); // Trackヘッダー
    }
    for _ in &app.form_fields {
        constraints.push(Constraint::Length(3));
    }
    constraints.push(Constraint::Min(0)); // 余白

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    // タイトル（アーティスト - 曲名）
    let title = Paragraph::new(format!("{} - {}", app.current_artist, app.current_track))
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
    frame.render_widget(title, chunks[0]);

    // Trackヘッダー（表示専用、j/kで選択不可）
    if has_header {
        let total = app.bpm_matches.len();
        let current = app.suggestion_index + 1;
        let track_title = if total > 1 {
            format!("Track {}/{} [a:Again]", current, total)
        } else {
            "Track".to_string()
        };
        let header = Paragraph::new(app.form_header.as_str())
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(track_title)
                    .border_style(Style::default().fg(Color::DarkGray)),
            );
        frame.render_widget(header, chunks[1]);
    }

    // フォームフィールド
    for (i, field) in app.form_fields.iter().enumerate() {
        let is_selected = i == app.form_index;
        let is_editing = is_selected && app.mode == Mode::Insert;

        let border_style = if is_editing {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else if is_selected {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };

        let title = if is_selected && app.mode == Mode::Normal {
            format!("> {}", field.label)
        } else {
            field.label.clone()
        };

        let chunk = chunks[i + 1 + header_len];

        if field.is_select() {
            // リスト選択型: 横並びの選択肢を表示
            let spans: Vec<Span> = field
                .options
                .iter()
                .enumerate()
                .flat_map(|(oi, opt)| {
                    let is_active = oi == field.selected;
                    let mut parts = Vec::new();
                    if oi > 0 {
                        parts.push(Span::raw("  "));
                    }
                    if is_active {
                        parts.push(Span::styled(
                            format!("[{}]", opt),
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ));
                    } else {
                        parts.push(Span::styled(
                            format!(" {} ", opt),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                    parts
                })
                .collect();

            let content = Paragraph::new(Line::from(spans)).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(border_style),
            );
            frame.render_widget(content, chunk);
        } else {
            // テキスト入力型
            let style = if is_selected {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };

            let display_value = if is_editing && field.value.is_empty() {
                "_".to_string()
            } else {
                field.value.clone()
            };

            let input = Paragraph::new(display_value)
                .style(style)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(title)
                        .border_style(border_style),
                );

            frame.render_widget(input, chunk);

            // カーソル表示
            if is_editing {
                frame.set_cursor_position((
                    chunk.x + 1 + display_width_up_to(&field.value, field.cursor),
                    chunk.y + 1,
                ));
            }
        }
    }

    // Genreフィールド(index=4)でサジェスト表示
    if app.form_index == 4 && !app.suggestions.is_empty()
        && (app.mode == Mode::Insert || app.selecting_suggestion)
    {
        let remaining_area = chunks.last().copied().unwrap_or(area);
        draw_suggestions(frame, app, remaining_area);
    }
}

/// 曲テーブル
fn draw_song_table(frame: &mut Frame, app: &App, area: Rect) {
    let header = Row::new(vec!["Artist", "Label", "Date", "Album", "Track", "Role", "Name", "Cnt"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .height(1);

    // list_offsetからvisible_rows分だけスライス
    let end = (app.list_offset + app.visible_rows).min(app.credits.len());
    let visible_songs = &app.credits[app.list_offset..end];

    let rows: Vec<Row> = visible_songs
        .iter()
        .enumerate()
        .map(|(i, song)| {
            let actual_index = app.list_offset + i;
            let style = row_style(app, actual_index);
            let role_str = song.role.clone().unwrap_or_default();
            let is_editing_album = app.editing_log_album && actual_index == app.list_index;
            let album_cell = if is_editing_album {
                Cell::from(format!(">{}", app.edit_buffer))
                    .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
            } else if song.is_aoty {
                Cell::from(song.album.clone().unwrap_or_default()).style(Style::default().fg(GOLD))
            } else {
                Cell::from(song.album.clone().unwrap_or_default())
            };
            let track_cell = if song.is_soty {
                Cell::from(song.track.clone()).style(Style::default().fg(GOLD))
            } else {
                Cell::from(song.track.clone())
            };
            Row::new(vec![
                Cell::from(song.artist.clone()),
                Cell::from(song.label.clone().unwrap_or_default()),
                Cell::from(song.date.clone().unwrap_or_default()),
                album_cell,
                track_cell,
                Cell::from(role_str.clone()).style(Style::default().fg(role_color(&role_str))),
                {
                    let name = song.name.clone().unwrap_or_default();
                    let s = if app.writer_data_names.contains(&name) {
                        Style::default().fg(Color::White)
                    } else {
                        Style::default()
                    };
                    Cell::from(name).style(s)
                },
                Cell::from(song.count.map(|c| c.to_string()).unwrap_or_default()),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(14), // Artist
            Constraint::Percentage(9),  // Label
            Constraint::Percentage(11), // Date
            Constraint::Percentage(14), // Album
            Constraint::Percentage(17), // Track
            Constraint::Percentage(10), // Role
            Constraint::Percentage(20), // Name
            Constraint::Percentage(5),  // Count
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Songs ({})", app.credits.len())),
    );

    frame.render_widget(table, area);
}

/// アーティストテーブル
fn draw_artist_table(frame: &mut Frame, app: &App, area: Rect) {
    // インライン編集中は別の描画関数を使う
    if app.mode == Mode::Insert && app.editing_field.is_some() {
        draw_artist_edit_view(frame, app, area);
        return;
    }

    let header = Row::new(vec!["Artist", "Label", "Memo"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .height(1);

    // list_offsetからvisible_rows分だけスライス
    let end = (app.list_offset + app.visible_rows).min(app.artists.len());
    let visible_artists = &app.artists[app.list_offset..end];

    let rows: Vec<Row> = visible_artists
        .iter()
        .enumerate()
        .map(|(i, artist)| {
            let actual_index = app.list_offset + i;
            let style = if actual_index == app.list_index && app.mode == Mode::Visual {
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
            } else {
                row_style(app, actual_index)
            };
            Row::new(vec![
                artist.artist.clone(),
                artist.label.clone().unwrap_or_default(),
                artist.memo.clone().unwrap_or_default(),
            ])
            .style(style)
        })
        .collect();

    // タイトルにモードヒントを追加
    let title = match app.mode {
        Mode::Visual => format!("Artists ({}) [j/k: move, d: delete, u: undo, Esc: exit]", app.artists.len()),
        Mode::Normal => format!("Artists ({}) [v: visual, e: edit]", app.artists.len()),
        _ => format!("Artists ({})", app.artists.len()),
    };

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(30),
            Constraint::Percentage(40),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(title),
    );

    frame.render_widget(table, area);
}

/// アーティストインライン編集ビュー
fn draw_artist_edit_view(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // ヘッダー
            Constraint::Min(0),     // リスト
        ])
        .split(area);

    // ヘッダー
    let header = Paragraph::new("Artist              | Label              | Memo")
        .style(Style::default().add_modifier(Modifier::BOLD));
    frame.render_widget(header, chunks[0]);

    // list_offsetからvisible_rows分だけスライス
    let end = (app.list_offset + app.visible_rows).min(app.artists.len());
    let start = app.list_offset;

    let list_area = chunks[1];
    let row_height = 1u16;

    for (i, idx) in (start..end).enumerate() {
        if let Some(artist) = app.artists.get(idx) {
            let y = list_area.y + i as u16 * row_height;
            if y >= list_area.y + list_area.height {
                break;
            }

            let is_current = idx == app.list_index;
            let row_area = Rect::new(list_area.x, y, list_area.width, row_height);

            if is_current && app.editing_field.is_some() {
                // 編集中の行
                let col_widths = [
                    list_area.width * 30 / 100,
                    list_area.width * 30 / 100,
                    list_area.width * 40 / 100,
                ];
                let mut x_offset = list_area.x;

                // Artist (編集不可)
                let artist_text = Paragraph::new(artist.artist.clone())
                    .style(Style::default().fg(Color::DarkGray));
                frame.render_widget(artist_text, Rect::new(x_offset, y, col_widths[0], 1));
                x_offset += col_widths[0];

                // Label
                let label_style = if app.editing_field == Some(0) {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let label_text = if app.editing_field == Some(0) {
                    format!(">{}", app.edit_buffer)
                } else {
                    artist.label.clone().unwrap_or_default()
                };
                let label = Paragraph::new(label_text).style(label_style);
                frame.render_widget(label, Rect::new(x_offset, y, col_widths[1], 1));

                // カーソル表示 (Label)
                if app.editing_field == Some(0) {
                    frame.set_cursor_position((
                        x_offset + 1 + display_width_up_to(&app.edit_buffer, app.edit_cursor),
                        y,
                    ));
                }
                x_offset += col_widths[1];

                // Memo
                let memo_style = if app.editing_field == Some(1) {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let memo_text = if app.editing_field == Some(1) {
                    format!(">{}", app.edit_buffer)
                } else {
                    artist.memo.clone().unwrap_or_default()
                };
                let memo = Paragraph::new(memo_text).style(memo_style);
                frame.render_widget(memo, Rect::new(x_offset, y, col_widths[2], 1));

                // カーソル表示 (Memo)
                if app.editing_field == Some(1) {
                    frame.set_cursor_position((
                        x_offset + 1 + display_width_up_to(&app.edit_buffer, app.edit_cursor),
                        y,
                    ));
                }
            } else {
                // 通常の行
                let style = if is_current {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };
                let text = format!(
                    "{:<20}| {:<18}| {}",
                    truncate_str(&artist.artist, 18),
                    truncate_str(&artist.label.clone().unwrap_or_default(), 16),
                    artist.memo.clone().unwrap_or_default()
                );
                let row = Paragraph::new(text).style(style);
                frame.render_widget(row, row_area);
            }
        }
    }
}

/// 文字列を指定長で切り詰め（文字数ベース）
fn truncate_str(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count > max_len {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}

/// ライターテーブル
fn draw_writer_table(frame: &mut Frame, app: &App, area: Rect) {
    let header = Row::new(vec!["Name", "RealName", "BirthDate", "BirthPlace", "Occupation", "Agency", "Debut", "Memo"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .height(1);

    // list_offsetからvisible_rows分だけスライス
    let end = (app.list_offset + app.visible_rows).min(app.writers.len());
    let visible_writers = &app.writers[app.list_offset..end];

    let rows: Vec<Row> = visible_writers
        .iter()
        .enumerate()
        .map(|(i, writer)| {
            let actual_index = app.list_offset + i;
            let style = row_style(app, actual_index);
            Row::new(vec![
                writer.name.clone(),
                writer.real_name.clone().unwrap_or_default(),
                writer.birth_date.clone().unwrap_or_default(),
                writer.birth_place.clone().unwrap_or_default(),
                writer.occupation.clone().unwrap_or_default(),
                writer.agency.clone().unwrap_or_default(),
                writer.debut.clone().unwrap_or_default(),
                writer.memo.clone().unwrap_or_default(),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(15),
            Constraint::Percentage(13),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(10),
            Constraint::Percentage(14),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Writers ({})", app.writers.len())),
    );

    frame.render_widget(table, area);
}

/// TrackDataテーブル
fn draw_track_table(frame: &mut Frame, app: &App, area: Rect) {
    let header = Row::new(vec!["Artist", "Label", "Date", "Album", "Track", "Genre", "Dur", "BPM", "Sp", "Rel", "A", "S"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .height(1);

    // list_offsetからvisible_rows分だけスライス
    let end = (app.list_offset + app.visible_rows).min(app.tracks.len());
    let visible_tracks = &app.tracks[app.list_offset..end];

    let rows: Vec<Row> = visible_tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let actual_index = app.list_offset + i;
            let style = row_style(app, actual_index);
            let duration = track
                .duration
                .map(|d| format!("{}:{:02}", d / 60, d % 60))
                .unwrap_or_default();
            let release = if track.is_title {
                "Title"
            } else if track.is_prerelease {
                "Pre"
            } else {
                "-"
            };
            let album_cell = if track.is_aoty {
                Cell::from(track.album.clone().unwrap_or_default()).style(Style::default().fg(GOLD))
            } else {
                Cell::from(track.album.clone().unwrap_or_default())
            };
            let track_cell = if track.is_soty {
                Cell::from(track.track.clone()).style(Style::default().fg(GOLD))
            } else {
                Cell::from(track.track.clone())
            };
            let genre_display = track.genres.as_ref()
                .map(|g| crate::models::genres_display(g))
                .unwrap_or_default();
            Row::new(vec![
                Cell::from(track.artist.clone()),
                Cell::from(track.label.clone().unwrap_or_default()),
                Cell::from(track.date.clone().unwrap_or_default()),
                album_cell,
                track_cell,
                Cell::from(genre_display),
                Cell::from(duration),
                Cell::from(track.bpm.clone().unwrap_or_default()),
                Cell::from(if track.spotify.as_ref().map_or(false, |s| !s.is_empty()) { "#".to_string() } else { String::new() }),
                Cell::from(release.to_string()),
                Cell::from(if track.is_aoty { "*" } else { "" }.to_string()),
                Cell::from(if track.is_soty { "*" } else { "" }.to_string()),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(11), // Artist
            Constraint::Percentage(7),  // Label
            Constraint::Percentage(9),  // Date
            Constraint::Percentage(11), // Album
            Constraint::Percentage(15), // Track
            Constraint::Percentage(12), // Genre
            Constraint::Percentage(5),  // Duration
            Constraint::Percentage(5),  // BPM
            Constraint::Percentage(3),  // Spotify
            Constraint::Percentage(5),  // Release
            Constraint::Percentage(4),  // AOTY
            Constraint::Percentage(4),  // SOTY
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(match app.track_filter {
                crate::tui::app::TrackFilter::Soty => format!("TrackData [SOTY] ({})", app.tracks.len()),
                crate::tui::app::TrackFilter::Aoty => format!("TrackData [AOTY] ({})", app.tracks.len()),
                crate::tui::app::TrackFilter::All => format!("TrackData ({})", app.tracks.len()),
            }),
    );

    frame.render_widget(table, area);
}

/// Writer検索結果
fn draw_writer_result(frame: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(11), // WriterData + Statistics + Yearly
            Constraint::Min(0),    // 曲リスト
        ])
        .split(area);

    // 上部を3分割: WriterData | Statistics | Yearly
    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(30),
            Constraint::Percentage(37),
        ])
        .split(chunks[0]);

    // WriterData情報
    let writer_text = if let Some(ref wd) = app.search_writer_data {
        let mut lines = Vec::new();
        lines.push(format!("Name: {}", wd.name));
        if let Some(ref v) = wd.real_name { lines.push(format!("Real Name: {}", v)); }
        if let Some(ref v) = wd.birth_date {
            let age_str = calc_age(v).map_or(String::new(), |a| format!(" ({})", a));
            lines.push(format!("Birth Date: {}{}", v, age_str));
        }
        if let Some(ref v) = wd.birth_place { lines.push(format!("Birth Place: {}", v)); }
        if let Some(ref v) = wd.occupation { lines.push(format!("Occupation: {}", v)); }
        if let Some(ref v) = wd.agency { lines.push(format!("Agency: {}", v)); }
        if let Some(ref v) = wd.debut { lines.push(format!("Debut: {}", v)); }
        if let Some(ref v) = wd.memo { lines.push(format!("Memo: {}", v)); }
        lines.join("\n")
    } else {
        "No writer data registered".to_string()
    };

    let writer_info = Paragraph::new(writer_text)
        .block(Block::default().borders(Borders::ALL).title("Writer Data"))
        .wrap(Wrap { trim: true });

    frame.render_widget(writer_info, top_chunks[0]);

    // 統計情報（表形式）
    let stats_map: std::collections::HashMap<&str, i64> = app.writer_stats.iter()
        .map(|(r, c)| (r.as_str(), *c))
        .collect();
    let rank_map: std::collections::HashMap<&str, i64> = app.writer_ranks.iter()
        .map(|(r, c)| (r.as_str(), *c))
        .collect();

    let role_rows = vec![
        ("lyricist", stats_map.get("lyricist").copied().unwrap_or(0)),
        ("composer", stats_map.get("composer").copied().unwrap_or(0)),
        ("arranger", stats_map.get("arranger").copied().unwrap_or(0)),
        ("writer", stats_map.get("writer").copied().unwrap_or(0)),
        ("total", app.writer_total_count),
        ("AOTY", app.writer_aoty_count),
        ("SOTY", app.writer_soty_count),
    ];

    let stats_header = Row::new(vec!["Role", "Sum", "Rank"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .height(1);

    let stats_rows: Vec<Row> = role_rows.iter().map(|(role, count)| {
        let rank = rank_map.get(role).copied().unwrap_or(0);
        Row::new(vec![
            Cell::from(role.to_string()).style(Style::default().fg(role_color(role))),
            Cell::from(count.to_string()),
            Cell::from(if *count > 0 { rank.to_string() } else { "-".to_string() }),
        ])
    }).collect();

    let stats_table = Table::new(
        stats_rows,
        [
            Constraint::Percentage(40),
            Constraint::Percentage(30),
            Constraint::Percentage(30),
        ],
    )
    .header(stats_header)
    .block(Block::default().borders(Borders::ALL).title("Statistics"));

    frame.render_widget(stats_table, top_chunks[1]);

    // Yearly棒グラフ
    let yearly_map: std::collections::HashMap<String, i64> = app.writer_yearly.iter().cloned().collect();

    let bars: Vec<Bar> = (2022..=2026)
        .map(|y| {
            let count = yearly_map.get(&y.to_string()).copied().unwrap_or(0);
            Bar::default()
                .value(count as u64)
                .label(Line::from(format!("({})", y % 100)))
                .style(Style::default().fg(Color::Cyan))
        })
        .collect();

    let yearly_chart = BarChart::default()
        .block(Block::default().borders(Borders::ALL).title("Yearly"))
        .data(BarGroup::default().bars(&bars))
        .bar_width(4)
        .bar_gap(1)
        .value_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

    frame.render_widget(yearly_chart, top_chunks[2]);

    // 曲リスト（実際の領域に合わせてvisible_rowsを再計算）
    let table_height = chunks[1].height as usize;
    if table_height > 3 {
        app.visible_rows = table_height - 3; // ボーダー2行 + ヘッダー1行
    }
    // スクロール位置を補正
    if app.list_index >= app.list_offset + app.visible_rows {
        app.list_offset = app.list_index - app.visible_rows + 1;
    }

    let header = Row::new(vec!["Artist", "Track", "Role", "Date"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .height(1);

    // list_offsetからvisible_rows分だけスライス
    let end = (app.list_offset + app.visible_rows).min(app.search_results.len());
    let visible_results = &app.search_results[app.list_offset..end];

    let rows: Vec<Row> = visible_results
        .iter()
        .enumerate()
        .map(|(i, song)| {
            let actual_index = app.list_offset + i;
            let style = row_style(app, actual_index);
            let role_str = song.role.clone().unwrap_or_default();
            let track_cell = if song.is_soty {
                Cell::from(song.track.clone()).style(Style::default().fg(GOLD))
            } else {
                Cell::from(song.track.clone())
            };
            Row::new(vec![
                Cell::from(song.artist.clone()),
                track_cell,
                Cell::from(role_str.clone()).style(Style::default().fg(role_color(&role_str))),
                Cell::from(song.date.clone().unwrap_or_default()),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(25),
            Constraint::Percentage(35),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Songs ({})", app.search_results.len())),
    );

    frame.render_widget(table, chunks[1]);
}

/// TrackDataブロックの描画ヘルパー（縦配置用）
fn draw_track_data_vertical(frame: &mut Frame, td: &crate::models::TrackData, credit: Option<&crate::models::CreditData>, artist_label: Option<&str>, area: Rect) {
    let album = credit.and_then(|c| c.album.as_deref()).unwrap_or("");
    let label = artist_label.unwrap_or("");
    let date = credit.and_then(|c| c.date.as_deref()).unwrap_or("");
    let dur = td.duration.map(|d| format!("{}:{:02}", d / 60, d % 60)).unwrap_or_default();
    let bpm = td.bpm.clone().unwrap_or_default();
    let release = if td.is_title { "Title" } else if td.is_prerelease { "Pre" } else { "-" };
    let aoty = if td.is_aoty { "*" } else { "-" };
    let soty = if td.is_soty { "*" } else { "-" };
    let spotify = if td.spotify.as_ref().map_or(false, |s| !s.is_empty()) { "#" } else { "" };
    let genre_display = td.genres.as_ref()
        .map(|g| crate::models::genres_display(g))
        .unwrap_or_default();

    let lines = vec![
        Line::from(vec![
            Span::styled("Album: ", Style::default().fg(Color::DarkGray)),
            if td.is_aoty { Span::styled(album, Style::default().fg(GOLD)) } else { Span::raw(album) },
        ]),
        Line::from(vec![
            Span::styled("Label: ", Style::default().fg(Color::DarkGray)),
            Span::raw(label),
        ]),
        Line::from(vec![
            Span::styled("Date:  ", Style::default().fg(Color::DarkGray)),
            Span::raw(date),
        ]),
        Line::from(vec![
            Span::styled("Genre: ", Style::default().fg(Color::DarkGray)),
            Span::raw(&genre_display),
        ]),
        Line::from(vec![
            Span::styled("Dur:   ", Style::default().fg(Color::DarkGray)),
            Span::raw(&dur),
        ]),
        Line::from(vec![
            Span::styled("BPM:   ", Style::default().fg(Color::DarkGray)),
            Span::raw(&bpm),
        ]),
        Line::from(vec![
            Span::styled("Rel:   ", Style::default().fg(Color::DarkGray)),
            Span::raw(release),
        ]),
        Line::from(vec![
            Span::styled("AOTY:  ", Style::default().fg(Color::DarkGray)),
            if td.is_aoty { Span::styled(aoty, Style::default().fg(GOLD)) } else { Span::raw(aoty) },
        ]),
        Line::from(vec![
            Span::styled("SOTY:  ", Style::default().fg(Color::DarkGray)),
            if td.is_soty { Span::styled(soty, Style::default().fg(GOLD)) } else { Span::raw(soty) },
        ]),
        Line::from(vec![
            Span::styled("Spotify: ", Style::default().fg(Color::DarkGray)),
            Span::raw(spotify),
        ]),
    ];

    let track_info = Paragraph::new(lines)
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("TrackData")
                .border_style(Style::default().fg(Color::DarkGray)),
        );
    frame.render_widget(track_info, area);
}

/// Song検索結果
fn draw_song_result(frame: &mut Frame, app: &mut App, area: Rect) {
    // 常に固定レイアウト: 上部(Album + TrackData) + 下部(Credits)
    let top_height: u16 = 13; // 11行 + ボーダー2行

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_height),
            Constraint::Min(0),
        ])
        .split(area);

    // 横分割: 左=Album, 中=TrackData, 右=Around-The-Day Drops
    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(22), // Album: 20文字 + ボーダー2
            Constraint::Fill(1),    // TrackData (均等)
            Constraint::Fill(1),    // Around-The-Day Drops (均等)
        ])
        .split(chunks[0]);

    // Albumブロック（アートがあれば256色カラー、なければ空欄）
    if let Some(ref art_lines) = app.album_art_current {
        let art_text: Vec<Line> = art_lines.iter().map(|row| {
            let spans: Vec<Span> = row.iter().map(|&(ch, r, g, b)| {
                Span::styled(
                    ch.to_string(),
                    Style::default().fg(Color::Indexed(rgb_to_256(r, g, b))),
                )
            }).collect();
            Line::from(spans)
        }).collect();
        let art = Paragraph::new(art_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Art")
                    .border_style(Style::default().fg(Color::DarkGray)),
            );
        frame.render_widget(art, top_chunks[0]);
    } else {
        draw_art_placeholder(frame, app, top_chunks[0]);
    }

    // TrackData（縦配置）
    if let Some(ref td) = app.search_track_data {
        draw_track_data_vertical(frame, td, app.search_results.first(), app.search_artist_label.as_deref(), top_chunks[1]);
    } else {
        let empty = Block::default()
            .borders(Borders::ALL)
            .title("TrackData")
            .border_style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty, top_chunks[1]);
    }

    // Around-The-Day Drops
    draw_around_day(frame, app, top_chunks[2], "Around-The-Day Drops");

    // Credits テーブル（実際の領域に合わせてvisible_rowsを再計算）
    let credit_area = chunks[1];
    let table_height = credit_area.height as usize;
    if table_height > 3 {
        app.visible_rows = table_height - 3;
    }
    if app.list_index >= app.list_offset + app.visible_rows {
        app.list_offset = app.list_index - app.visible_rows + 1;
    }

    let header = Row::new(vec!["Role", "Name", "Count"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .height(1);

    let end = (app.list_offset + app.visible_rows).min(app.search_results.len());
    let visible_results = &app.search_results[app.list_offset..end];

    let rows: Vec<Row> = visible_results
        .iter()
        .enumerate()
        .map(|(i, song)| {
            let actual_index = app.list_offset + i;
            let style = row_style(app, actual_index);
            let role_str = song.role.clone().unwrap_or_default();
            let name = song.name.clone().unwrap_or_default();
            let name_style = if app.writer_data_names.contains(&name) {
                Style::default().fg(Color::White)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(role_str.clone()).style(Style::default().fg(role_color(&role_str))),
                Cell::from(name).style(name_style),
                Cell::from(song.count.map(|c| c.to_string()).unwrap_or_default()),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(50),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title("Credits"));

    frame.render_widget(table, credit_area);
}

/// Around-The-Day Drops描画
fn draw_around_day(frame: &mut Frame, app: &App, area: Rect, title: &str) {
    let mut lines: Vec<Line> = Vec::new();

    for (md, items) in &app.around_day_drops {
        // MM-DDヘッダー
        lines.push(Line::from(Span::styled(
            md.as_str(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )));
        if items.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (none)",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for (year, artist, track) in items {
                lines.push(Line::from(format!("  {} {} - {}", year, artist, track)));
            }
        }
    }

    let block = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(Color::DarkGray)),
        );
    frame.render_widget(block, area);
}

/// Art枠のプレースホルダー（Loading or 空）
fn draw_art_placeholder(frame: &mut Frame, app: &App, area: Rect) {
    if app.album_art_receiver.is_some() {
        let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let ch = spinner[app.tick % spinner.len()];
        let inner_height = area.height.saturating_sub(2) as usize;
        let pad = inner_height / 2;
        let mut lines: Vec<Line> = (0..pad).map(|_| Line::from("")).collect();
        lines.push(Line::from(Span::styled(
            format!("{} Loading", ch),
            Style::default().fg(Color::DarkGray),
        )));
        let art = Paragraph::new(lines)
            .alignment(ratatui::layout::Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Art")
                    .border_style(Style::default().fg(Color::DarkGray)),
            );
        frame.render_widget(art, area);
    } else {
        let art = Block::default()
            .borders(Borders::ALL)
            .title("Album")
            .border_style(Style::default().fg(Color::DarkGray));
        frame.render_widget(art, area);
    }
}

/// RGBを256色パレットのインデックスに変換（6x6x6カラーキューブ: 16-231）
fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    let r6 = ((r as u16 * 5 + 127) / 255) as u8;
    let g6 = ((g as u16 * 5 + 127) / 255) as u8;
    let b6 = ((b as u16 * 5 + 127) / 255) as u8;
    16 + 36 * r6 + 6 * g6 + b6
}

/// Quizヘッダー
fn draw_quiz_header(frame: &mut Frame, question: usize, total: usize, score: usize, mode: Mode, area: Rect) {
    let mode_str = match mode {
        Mode::Normal => "[NORMAL]",
        Mode::Insert => "[INSERT]",
        Mode::Search => "[SEARCH]",
        Mode::Visual => "[VISUAL]",
    };
    let header = Paragraph::new(Line::from(vec![
        Span::styled("Quiz", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!(" ({}/{})  ", question, total)),
        Span::styled(format!("Score: {}", score), Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled(mode_str, Style::default().fg(Color::Yellow)),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(header, area);
}

/// Quiz正誤結果画面
fn draw_quiz_result(frame: &mut Frame, app: &mut App, area: Rect) {
    // 上: Result(全幅)  下: Art + TrackData
    let result_height: u16 = if app.quiz_last_correct { 5 } else { 6 };
    let art_height: u16 = 12;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(result_height),
            Constraint::Length(art_height),
            Constraint::Min(0),
        ])
        .split(area);

    // Result（正誤表示）全幅
    let mark = if app.quiz_last_correct { "O" } else { "X" };
    let mark_color = if app.quiz_last_correct { Color::Green } else { Color::Red };

    let mut lines = vec![
        Line::from(vec![
            Span::raw("  "),
            Span::styled(mark, Style::default().fg(mark_color).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled("Score: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{} / {}", app.quiz_score, app.quiz_current + 1),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    if !app.quiz_last_correct {
        lines.push(Line::from(vec![
            Span::styled("  Your Answer: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.quiz_last_answer, Style::default().fg(Color::Red)),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("  Correct:     ", Style::default().fg(Color::DarkGray)),
        Span::styled(&app.quiz_last_actual, Style::default().fg(Color::Green)),
    ]));

    let result_block = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Result"));
    frame.render_widget(result_block, chunks[0]);

    // 横分割: Art(22) + TrackData(残り)
    let bottom_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(22),
            Constraint::Min(0),
        ])
        .split(chunks[1]);

    // Album Art
    if let Some(ref art_lines) = app.album_art_current {
        let art_text: Vec<Line> = art_lines.iter().map(|row| {
            let spans: Vec<Span> = row.iter().map(|&(ch, r, g, b)| {
                Span::styled(
                    ch.to_string(),
                    Style::default().fg(Color::Indexed(rgb_to_256(r, g, b))),
                )
            }).collect();
            Line::from(spans)
        }).collect();
        let art = Paragraph::new(art_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Art")
                    .border_style(Style::default().fg(Color::DarkGray)),
            );
        frame.render_widget(art, bottom_chunks[0]);
    } else {
        draw_art_placeholder(frame, app, bottom_chunks[0]);
    }

    // TrackData
    if let Some(ref td) = app.search_track_data {
        draw_track_data_vertical(frame, td, app.search_results.first(), app.search_artist_label.as_deref(), bottom_chunks[1]);
    } else {
        let empty = Block::default()
            .borders(Borders::ALL)
            .title("TrackData")
            .border_style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty, bottom_chunks[1]);
    }
}

/// Quiz最終結果画面
fn draw_quiz_final(frame: &mut Frame, app: &App, area: Rect) {
    let total = app.quiz_questions.len();
    let score = app.quiz_score;
    let pct = if total > 0 { score * 100 / total } else { 0 };

    let grade = match pct {
        90..=100 => ("S", Color::Rgb(255, 215, 0)),
        80..=89 => ("A", Color::Green),
        70..=79 => ("B", Color::Cyan),
        60..=69 => ("C", Color::Yellow),
        _ => ("D", Color::Red),
    };

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  Final Score: {} / {}", score, total),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Grade: ", Style::default().fg(Color::DarkGray)),
            Span::styled(grade.0, Style::default().fg(grade.1).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Press Enter or Esc to return to menu",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    // 全問の結果一覧を表示
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Questions:",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for (i, (artist, track, _)) in app.quiz_questions.iter().enumerate() {
        let answered = i < app.quiz_current + 1; // quiz_currentは0-indexedで最後の問題のインデックス
        if !answered { break; }
        let prefix = format!("  {}. ", i + 1);
        lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(Color::DarkGray)),
            Span::raw(format!("{} - {}", artist, track)),
        ]));
    }

    let block = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Quiz Complete"));
    frame.render_widget(block, area);
}

/// サジェスチョンを表示すべきかどうか
fn should_show_suggestions(app: &App) -> bool {
    if app.suggestions.is_empty() {
        return false;
    }

    match &app.screen {
        // InputCreditData: Artistフィールド(form_index==0)のみ
        Screen::InputCreditData => app.form_index == 0,
        // InputArtistData: Artist(0)またはLabel(1)フィールド
        Screen::InputArtistData => app.form_index <= 1,
        // InputWriterData: Nameフィールド(form_index==0)のみ
        Screen::InputWriterData => app.form_index == 0,
        // その他は常に表示
        _ => true,
    }
}
