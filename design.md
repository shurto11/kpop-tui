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

## [追加機能] 自動追加
- InputCreditDataをspotifyから自動で入力されるように

### 前提条件
- 曲の対象はお気に入りの曲
- spotatuiがデバイスで使える

### 機能要求
- ユーザーがMenu-Inputからこのモードを選択できるように
- Inputの中では一番上に
- dataのlogを見て、一番新しい曲がお気に入りに含まれているか確かめる
- 含まれていたら、それ以降に追加された曲名とアーティスト名を表示する
- 一番新しい曲がお気に入りでも一番新しかったら、「追加する曲はない」ということを英語で表示
- 含まれていなかったら、お気に入りのすべてを表示
- geniusのリンクを作り、そのページが実際にあるか確かめる
- ネットに接続できないならエラー表示
- あったならマルを、ないならバツを表示する
- バツはユーザーが、曲名またはアーティスト名を編集できるようにする
- 曲自体を削除もできるように
- 全部マルになったら、ユーザーがEnterを押すとDataに追加
- 追加する際は、お気に入りの古い順に追加

### 実装 (Input > AutoAdd)

#### Spotify連携
認証は**spotatuiに任せ、kpop-tuiは読むだけ**。取得の直前に `spotatui list --liked --limit 1`
を呼んで期限切れならspotatui自身にリフレッシュさせ、`~/.config/spotatui/.spotify_token_cache*.json`
（mtime最新）から access_token を読む。**認証ファイルへの書き戻しはしない**
（PKCEのrefresh_tokenはローテーションするため、書き戻すとspotatui側の認証が壊れ得る）。
取得は `GET /v1/me/tracks` を `next` で辿って全件（limitの上限は50）。追加クレートは不要。

#### Genius URL の候補
Spotifyの曲名・アーティスト名はそのままではGeniusのスラッグに合わないため、
段階的に削って**最大5本**試し、最初に200が返ったものを採用する。

| # | 変換 | 例 |
|---|------|-----|
| 1 | そのまま | |
| 2 | アーティストの括弧を削除 | `ALL(H)OURS` → `allhours` |
| 3 | `(feat. X)` を除去 | |
| 4 | 曲名の ` - ...` 以降を除去 | `SIGN - Japanese Ver.` → `SIGN` |
| 5 | 末尾の括弧グループを除去 | `Touch (Y2K Unit)` → `Touch` |
| 6 | 主アーティストのみ | `A & B` → `A` |
| — | 各候補でアクセント除去も試す | `México` → `mexico` |

#### 画面の状態と操作
| キー | 動作 |
|------|------|
| `j`/`k` | 行移動 |
| `e` | Trackをインライン編集（編集中の `Tab` でArtistへ） |
| `d` | 行を候補から外す（DBには触らない） |
| `r` | 全行を再チェック |
| `o` | 当たったGeniusページをブラウザで開く |
| `Enter` | 全行をcredit_dataへ一括追加 |
| `Esc` | 取得・チェック中なら中断、そうでなければ戻る |

行のマーク: `[✓]` あり / `[✗]` 全候補が404 / `[!]` ネットワークエラー /
`[dup]` 既にcredit_dataにある / `[✗] artist not registered` artist_data未登録。

**✗ だった値は ✓ になっても消さない。** 手で直した場合も候補URLの自動リトライで当たった場合も、
外れた曲名（名前が変わっていなければ外れたURLのスラッグ）を `✗ was: ...` として行に残す。

#### 安全策
- artist_dataに未登録のアーティストが1つでもあれば、Enterでの一括追加を**ブロック**する。
  判定は必ずGenius側の名前で行う（実際にDBへ入るのがその名前のため）。
- credit_dataにはUNIQUE制約がないので、候補を作る時点で `credit_exists` を全行に走らせ
  既存曲を `[dup]` にして追加対象から外す。「log最新1曲で切る」判定が、後から古い曲を
  手動で足したときに崩れる弱点の安全弁になっている。
- 追加はトランザクションで一括INSERTし、`update_writer_count` は
  distinctなライター名について1回ずつだけ呼ぶ（クレジット毎に呼ぶと実質O(n²)）。
- Geniusが date / album を拾えなかった行は、Spotifyの `album.name` / `album.release_date` で補う。

#### 実装上の制約
- **DB操作はすべてメインスレッド。** `rusqlite::Connection` は `!Sync` なので
  `Arc<Database>` をワーカーに渡せない。ワーカーはHTTPとプロセス実行のみ。
- **`app.loading` は使わない。** `loading` 中は `Esc` 以外の全キーが捨てられるため、
  逐次○×更新しながら操作する画面と両立しない。進行は `auto_add_phase` と行ごとのスピナーで見せる。
- 結果の配送は配列インデックスではなく**行の安定ID + seq**で行う。
  これがないとスキャン中の削除・編集で別の行に○×が付く。
- ワーカーは `Arc<AtomicBool>` で中断できるようにし、`go_to`/`go_back`/`Esc` で必ず立てる。
- `scrape_genius` にステータスコード判定とタイムアウトを追加した。
  Geniusは存在しない曲にも404と一緒に長いHTMLを返すため、ステータスを見ないと
  404ページのタイトルから偽のartist/trackを拾う。
- `spotatui` の呼び出しには自前で15秒の上限を設ける（spotatui側にネットワークタイムアウトがない）。
