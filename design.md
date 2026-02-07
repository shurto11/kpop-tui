# コンセプト
Writerを重視したkpop曲の自作データから、tuiモードでテーブル形式で表示する

# データベース
SongData
    - Artist
    - Lable
    - Date
    - Album
    - Track
    - Role
    - Name
    - Count

SongAddData(これは正規化するかおまかせする)
    - Duration
    - BPM
    - Spotify
    - Track (true/false)
    - Pre-release(true/false)
    - Best16(true/false)

ArtistData
    - Artist
    - Label
    - Memo

WriterData
    - Name
    - RealName
    - BirthDate
    - BirthPlace
    - Occupation
    - Agency
    - Debut
    - Memo
    - Instagram
    - X
    - Spotify

# 仕様
    - Input
    - Search
    - View

## Input
ユーザーはどのデータに書き込むのか入力する

- SongDataの場合
 ユーザーはArtist, Trackを入力する
 https://genius.comからスクレイピングする。
 アドレスは、genius.com/artist-track-lyricsという規則性がある。
 スクレイピングで、Artist, Date, Album, Track, Role, Nameの情報を集め、データベースに書き込む。
 Num ArtistDataの昇順に1から番号をつける   
 Label ArtistDataから抽出
 Count NameがあるSong数を出力
 ArtistDataの情報がない場合、ArtistDataの場合に飛ばす

- SongAddDataの場合
 ユーザーは、曲ごとにDuration, BPM, Spotify, Track, Pre-releaseを入力する
 Duration, Spotifyはできるならスクレイピングしたい。適切なサイトがあるかは不明。
 他の項目は、ユーザーが入力する。(特にBPMは、サイトに嘘が記載されているので気をつけること)

- ArtistDataの場合
 ユーザーは、Artistを入力する。または並べ替え
 そのアーティストのLabel, Memoを入力する。
 Labelは他のアーティストのものからコピーできるように？(表記揺れを避けたい)
 Dataは同じLabelどおしは、連続させたい。
 Labelの順番は並べ替えでユーザーが決める

- WriterDataの場合
 ユーザーはNameを入力する
 Nameの他の項目をユーザーが入力する。スクレイピングではない。
 Instagram, x, spotifyはリンクである

- Best16の場合
 ユーザーはTrackを入力する？選択する？
 SongDataのBest16をTrue/False切り替える

## Search
何を探すのか入力する

- WriterSearch
 ユーザーは、Writerを入力する。(入力を補助できる仕組みもいれたい)
 入力された名前をSongDataから検索する
 Role (lyricist, composer, arrangest, writer)を表示し、その下にそれぞれTrackとArtistを表示する。
 Roleごとに合計何曲かとWriterの中の順位を表示する。
 Best16に何曲入っているかと順位を表示する。
 年ごとに何曲入っているかと順位を表示する。
 WriterDataの内容を出力する。

- SongSearch
 ユーザーは、ArtistとTrackを入力する。(Artistを入れたとき、Trackをリストにするなど入力の補助をしたい)
 RoleごとにWriterを表示する。
 Writerは何曲入っているか表示する。
 SongData, SongAddData, ArtistDataを出力する

## View
何を見るのか入力する
- Log
 SongDataに正しく入力されているかを確かめる。
 入力された順のまま並び替えない。
 上から新しいものである

- SongData
 こちらはArtistの昇順, 年月日の昇順に並び替える。

- ArtistData

- WriterData

- Best16
 Best16に選ばれた曲の、Track, Artist, Dateを出力する

# 注意点
- WriterとRoleのwriterは別です
- 何曲かと順位の集計は、Roleごとに分けるものと、複数Role関わっていても１曲としてまとめるものもあります

---

# 技術仕様

## 技術スタック
| コンポーネント | 技術 |
|--------------|------|
| 言語 | Rust |
| TUI | ratatui + crossterm |
| データベース | SQLite (rusqlite) |
| HTTP | reqwest |
| HTMLパース | scraper |
| 非同期 | tokio |

## ディレクトリ構成
```
~/my_cli/kpop4/
├── kpop4                # ビルド済みバイナリ
├── kpop.db              # SQLiteデータベース
├── config.toml          # 設定ファイル
├── src/
│   ├── main.rs
│   ├── db/              # データベース操作
│   ├── tui/             # TUI画面
│   ├── scraper/         # Geniusスクレイピング
│   └── models/          # データ構造
└── Cargo.toml
```

## データベーススキーマ (SQLite)

```sql
-- 曲データ（Geniusからスクレイピング）
CREATE TABLE song_data (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    artist TEXT NOT NULL,
    label TEXT,
    date TEXT,
    album TEXT,
    track TEXT NOT NULL,
    role TEXT,           -- lyricist/composer/arranger/writer
    name TEXT,           -- クレジット者名
    count INTEGER,       -- そのNameの総曲数
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- 曲追加データ
CREATE TABLE song_add_data (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    track TEXT NOT NULL,
    artist TEXT NOT NULL,
    duration INTEGER,    -- 秒数
    bpm INTEGER,
    spotify TEXT,        -- URL
    is_track BOOLEAN DEFAULT TRUE,
    is_prerelease BOOLEAN DEFAULT FALSE,
    is_best16 BOOLEAN DEFAULT FALSE,
    UNIQUE(track, artist)
);

-- アーティストデータ
CREATE TABLE artist_data (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    artist TEXT UNIQUE NOT NULL,
    label TEXT,
    memo TEXT,
    sort_order INTEGER   -- 表示順
);

-- ライターデータ
CREATE TABLE writer_data (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE NOT NULL,
    real_name TEXT,
    birth_date TEXT,
    birth_place TEXT,
    occupation TEXT,
    agency TEXT,
    debut TEXT,
    memo TEXT,
    instagram TEXT,
    x TEXT,
    spotify TEXT
);

-- インデックス
CREATE INDEX idx_song_artist ON song_data(artist);
CREATE INDEX idx_song_track ON song_data(track);
CREATE INDEX idx_song_name ON song_data(name);
CREATE INDEX idx_song_role ON song_data(role);
CREATE INDEX idx_song_date ON song_data(date);
```

## 設定ファイル (config.toml)

```toml
[genius]
header_key = "908aafe9"
info_key = "56e36c75"
date_key = "hFYGNw"

[database]
path = "kpop.db"
```

## スクレイピング仕様

### URL生成ロジック
1. 韓国語削除 (U+AC00-U+D7A3)
2. `()` を空白に変換
3. `. , : '` を削除
4. `&` → `and`
5. `æ` 削除
6. 空白 → `-`
7. 連続`-` を1つに
8. 先頭末尾の`-` を削除
9. 小文字化
10. 最終形: `https://genius.com/{artist}-{song}-lyrics`

### Geniusから取得する情報
| 項目 | CSSセレクタ |
|------|-------------|
| アーティスト | `SongHeader-desktop__CreditList-sc-{header_key}-16 a` |
| 曲名 | `SongHeader-desktop__Title-sc-{header_key}-9 span` |
| リリース日 | `LabelWithIcon__Label-sc-a1922d73-1.{date_key}` |
| アルバム | `a[href="#primary-album"]` |
| クレジット | `SongInfo__Credit-sc-{info_key}-3` |

### クレジット処理ルール
- 取得対象: lyricist, composer, arranger, writer
- 特殊ルール: composerがある場合 → writerは空にする

## TUI画面遷移

```
メインメニュー
├── Input
│   ├── SongData      → Artist, Track入力 → Geniusスクレイピング
│   ├── SongAddData   → 曲選択 → Duration/BPM等入力
│   ├── ArtistData    → Artist入力 → Label/Memo入力
│   ├── WriterData    → Name入力 → プロフィール入力
│   └── Best16        → 曲選択 → ON/OFF切替
│
├── Search
│   ├── WriterSearch  → Name入力 → Role別曲一覧＋統計
│   └── SongSearch    → Artist選択 → Track選択 → 詳細表示
│
└── View
    ├── Log           → 入力順一覧（新しい順）
    ├── SongData      → Artist順 → 日付順
    ├── ArtistData    → Label順
    ├── WriterData    → 一覧
    └── Best16        → Best16一覧
```

## TUI操作（Vimキーバインド）

### ノーマルモード

| キー | 動作 |
|------|------|
| `j` / `↓` | 下に移動 |
| `k` / `↑` | 上に移動 |
| `h` / `←` | 左に移動 / 前の画面 |
| `l` / `→` / `Enter` | 右に移動 / 選択決定 |
| `gg` | 先頭に移動 |
| `G` | 末尾に移動 |
| `Ctrl+d` | 半ページ下 |
| `Ctrl+u` | 半ページ上 |
| `/` | 検索モード開始 |
| `n` | 次の検索結果 |
| `N` | 前の検索結果 |
| `q` / `Esc` | 戻る / 終了 |

### インサートモード（テキスト入力時）

| キー | 動作 |
|------|------|
| `Esc` | ノーマルモードに戻る |
| `Enter` | 入力確定 |
| `Ctrl+c` | 入力キャンセル |
| `Tab` | 補完候補を選択 |

### 補完機能
- Artist/Track/Name入力時にインクリメンタル補完
- `Tab` で候補を選択、`Shift+Tab` で逆順

### 画面固有キー

| 画面 | キー | 動作 |
|------|------|------|
| View系 | `s` | ソート切替 |
| View系 | `f` | フィルタ設定 |
| Best16 | `Space` | ON/OFF切替 |
| 詳細表示 | `o` | URLを開く（Spotify等） |
