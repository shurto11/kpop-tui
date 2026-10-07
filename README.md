# kpop-tui

作詞・作曲・編曲などの**作家クレジット**を軸に、K-POP曲の自作データベースを管理するターミナルアプリ。
Genius からクレジットを、songbpm.com から BPM を取り込み、Writer ごとの曲数・順位・年別推移をテーブルで眺められる。

- Vim ライクなキー操作の TUI（ratatui + crossterm）
- データは SQLite（`kpop.db`）1ファイル
- Spotify のお気に入りから新しい曲をまとめて取り込む **AutoAdd**
- spotatui 経由で曲を再生しながら閲覧、イントロ当ての **Quiz**
- 同じ DB を読み取り専用で開くブラウザ版 `kpop-web`（[web/README.md](web/README.md)）

## 必要なもの

| 用途 | 必要なもの |
| --- | --- |
| ビルド | Rust（edition 2021） |
| AutoAdd・再生・Quiz | [spotatui](https://github.com/LargeModGames/spotatui)（認証済みであること） |
| 終了時の SQL ダンプ push | `python3`、`git`、データ用リポジトリ `kpop-tui-data` |
| ブラウザ版の公開 | Tailscale |

spotatui が無くても、手動入力・検索・閲覧は使える。

## インストール

```bash
git clone git@github.com:shurto11/kpop-tui.git ~/ssd/tui/kpop-tui
cd ~/ssd/tui/kpop-tui
cargo install --path .     # kpop-tui と kpop-web が ~/.cargo/bin に入る
kpop-tui
```

### データの置き場所

データディレクトリは `~/ssd/tui/kpop-tui/` に固定されている（`src/main.rs` と `src/bin/web.rs` の `get_data_dir()`）。
どこから起動しても、以下はこのディレクトリから読み書きされる。

```
~/ssd/tui/kpop-tui/
├── config.toml              設定（初回起動時にデフォルト値で自動生成）
├── kpop.db                  SQLite データベース
├── backup/                  終了時に書き出される CSV
└── scripts/backup-data.sh   終了時に実行される SQL ダンプ push
```

別の場所で使う場合は `get_data_dir()` を書き換える。

## 設定（config.toml）

```toml
[genius]
header_key = "..."   # 曲ヘッダー部分の CSS クラスのハッシュ
info_key   = "..."   # 曲情報部分
date_key   = "..."   # リリース日ラベル
credit_key = "..."   # クレジット欄（Credit__Container 等）

[database]
path = "kpop.db"     # データディレクトリからの相対パス
```

`[genius]` のキーは Genius のページに付いている CSS クラス名のハッシュ部分。
Genius 側の更新で変わることがあり、そのときはスクレイピングで情報が取れなくなるので、
ブラウザの開発者ツールで新しいクラス名を確認して書き換える。

## 画面構成

```
MainMenu ─ Today's Drops
├── Input
│   ├── AutoAdd      Spotify のお気に入りから一括追加
│   ├── CreditData   Artist / Track を入力して Genius から取り込み
│   ├── TrackData    BPM・長さ・Spotify URL・Release・Genre を入力
│   ├── ArtistData   Label / Memo
│   ├── WriterData   作家のプロフィール
│   └── WriterAka    同一人物の別名を紐付け
├── Search
│   ├── WriterSearch 作家ごとの統計と曲リスト
│   └── TrackSearch  曲ごとのクレジットと TrackData
├── View
│   ├── Log          入力順（新しい順）
│   ├── CreditData   Artist 順 → 日付順
│   ├── TrackData    全件 / SOTY / AOTY
│   ├── ArtistData   Label 順（並べ替え可）
│   └── WriterData
└── Quiz             10 問のイントロ当て
```

### MainMenu（Today's Drops）

今日と同じ月日にリリースされた曲を1曲ランダムに選び、クレジット・TrackData・
Spotify のジャケット（ASCII アート）を表示する。該当曲がなければ最新の曲を表示する。
あわせて、その曲のリリース日の前後1日に出た曲を **Around-The-Day Drops** として並べる。

### Input

- **AutoAdd** — Spotify のお気に入りのうち、Log の最新曲より後に追加された曲を候補にする
  （最新曲がお気に入りに無ければ全件）。各曲について:
  1. Genius のページがあるか確認する。URL の候補を段階的に崩して最大5本試す
     （括弧・`feat.`・` - Japanese Ver.` などを除去、アクセント除去）。
  2. songbpm.com から BPM 候補を取り、Spotify ID が一致するものを優先して並べる。

  行の状態は `[✓]` あり / `[✗]` 全候補が 404 / `[!]` ネットワークエラー /
  `[dup]` 登録済み / `artist not registered` ArtistData 未登録。
  ✗ の行は Track・Artist を手で直すか、候補から外す。
  `Enter` で、お気に入りに入れた古い順に CreditData と TrackData へまとめて追加する。
  ArtistData 未登録のアーティストが残っている間は追加できない。
- **CreditData** — Artist と Track を入力すると Genius をスクレイピングし、
  lyricist / composer / arranger / writer のクレジットを1人1行で登録する。
  アーティストが ArtistData に無ければ、そのまま ArtistData の入力に移る。
- **TrackData** — TrackData が未入力の曲を Artist → Track の順に選ぶと、
  songbpm.com から Duration・BPM・Spotify URL を取得してフォームに入れる。
  BPM は「半分 / そのまま / 倍 / MIXX」から選ぶ（サイトの値は倍・半分にずれていることがあるため）。
- **ArtistData** — レーベルとメモ。
- **WriterData** — 本名・生年月日・出身・職業・所属・デビュー・メモ。
- **WriterAka** — クレジット名どうしで単語が一致するペアを一覧にし、`Space` で同一人物として紐付ける。
  紐付けた名前は WriterSearch で合算される。

### Search

- **WriterSearch** — Role 別の曲数と作家内での順位、SOTY / AOTY の曲数、年別の曲数、
  WriterData、関わった曲のリストを表示する。
- **TrackSearch** — Artist → Track を補完付きで選び、Role ごとの作家とその曲数、TrackData を表示する。

### Quiz

Spotify URL のある曲からランダムに10問出題し、自動で再生する。
Artist → Track を補完から選んで答える。`p` でパス。

## キー操作

### 共通（ノーマルモード）

| キー | 動作 |
| --- | --- |
| `j` / `k` | 下 / 上に移動 |
| `l` / `Enter` | 選択・決定 |
| `h` / `Esc` | 戻る |
| `gg` / `G` | 先頭 / 末尾へ |
| `Ctrl+d` / `Ctrl+u` | 半ページ下 / 上 |
| `/` → `n` / `N` | 一覧内を検索 → 次 / 前の一致 |
| `i` | 入力モード（入力画面） |
| `c` / `x` | カーソル位置の曲を Spotify で再生 / 一時停止 |
| `←` / `→`（再生中） | 10秒戻す / 進める |
| `q` | MainMenu に戻る |
| `Q` / `Ctrl+C` | 終了 |

### 入力モード

| キー | 動作 |
| --- | --- |
| `Tab` | 補完候補を選ぶ |
| `Enter` | 確定 |
| `Esc` | ノーマルモードへ |
| `Ctrl+C` | 入力を取り消す |

### 画面ごと

| 画面 | キー |
| --- | --- |
| AutoAdd | `e` 編集（編集中 `Tab` で Track ⇄ Artist）、`d` 候補から外す、`r` 再チェック、`R` BPM 再取得、`o` Genius を開く、`Tab` 下の TrackData 枠へ（`j`/`k` で項目、`h`/`l` で候補・BPM・Release を選択）、`Enter` 一括追加、`Esc` 取得中なら中断 |
| TrackData 入力 | `a` 次の BPM 候補、`r` BPM 再取得、`R` 別のアーティスト名で再取得、`d` フィールドを空にする |
| Log | `e` アルバム名を編集、`d` 曲を削除、`r` 削除を取り消す、`l` 作家へ |
| CreditData | `l` 作家へ |
| TrackData | `l` 詳細、`e` 編集、`s` / `a` SOTY / AOTY で絞り込み、`S` / `A` SOTY / AOTY を切り替え |
| ArtistData | `v` ビジュアルモード（`j`/`k` で並べ替え、`d` 削除）、`e` 編集、`u` / `Ctrl+r` 元に戻す / やり直す |
| WriterData | `l` 曲一覧、`e` 編集 |
| WriterSearch 結果 | `l` 曲の詳細、`e` WriterData を編集 |
| WriterAka | `Space` 紐付けの切り替え、`d` ペアを一覧から外す |
| Quiz | `i` 回答入力、`p` パス |

画面下のフッターに、その画面で使えるキーが常に表示される。

## データベース

| テーブル | 内容 |
| --- | --- |
| `credit_data` | クレジット。1曲 × 1作家 × 1 Role で1行（artist, label, date, album, track, role, name, count） |
| `track_data` | 曲ごとの追加情報（duration, bpm, spotify, is_title, is_prerelease, is_soty, is_aoty, genres） |
| `artist_data` | アーティスト（label, memo, sort_order） |
| `writer_data` | 作家のプロフィール |
| `writer_aka` | 同一人物の別名（primary_name ↔ alias_name） |
| `aka_dismissed` | WriterAka で一覧から外したペア |

スキーマは起動時に自動で作成・マイグレーションされる。

## CLI

```bash
kpop-tui                          # TUI を起動
kpop-tui --export [dir]           # 全テーブルを CSV に書き出す（既定: backup/）
kpop-tui --import-credit  <csv>   # CSV から credit_data に取り込む
kpop-tui --import-track   <csv>   # track_data
kpop-tui --import-artist  <csv>   # artist_data
kpop-tui --import-writer  <csv>   # writer_data
```

## バックアップ

TUI を終了すると、自動で次の2つが走る。

1. `backup/` に全テーブルの CSV を書き出す（`credit.csv` `track.csv` `artist.csv` `writer.csv` `writer_aka.csv`）。
2. `scripts/backup-data.sh` があれば実行し、`kpop.db` の SQL ダンプを
   隣のディレクトリの `kpop-tui-data` リポジトリにコミットして push する（変更があるときだけ）。

`backup-data.sh` の場所は環境変数で変えられる。

| 変数 | 既定値 |
| --- | --- |
| `KPOP_DB` | `<データディレクトリ>/kpop.db` |
| `KPOP_DATA_REPO` | `<データディレクトリの親>/kpop-tui-data` |

SQL ダンプからの復元:

```bash
python3 -c "import sqlite3; sqlite3.connect('kpop.db').executescript(open('kpop.sql').read())"
```

## ブラウザ版（kpop-web）

TUI と同じ `kpop.db` を読み取り専用で開き、スマホなどから閲覧・検索できる。
Tailscale で tailnet 内だけに公開する。

```bash
./scripts/kpop-web.sh          # 127.0.0.1:8787 で起動して tailscale serve
./scripts/kpop-web.sh --stop   # 公開を解除
```

画面・API の詳細は [web/README.md](web/README.md)。入力系（Input・AutoAdd・Quiz）は TUI 版のみ。

## 外部サービス

| サービス | 使い方 |
| --- | --- |
| Genius | 曲ページをスクレイピングしてクレジット・アルバム・リリース日を取得 |
| songbpm.com | BPM・曲の長さ・Spotify URL の候補を取得 |
| Spotify Web API | お気に入り一覧の取得（AutoAdd）。トークンは spotatui のキャッシュを読むだけで、書き戻さない |
| Spotify oEmbed | ジャケット画像の URL を取得 |
| spotatui | 曲の再生・一時停止・シーク |

## ディレクトリ構成

```
src/
├── main.rs        bin: kpop-tui（起動、イベントループ、終了時バックアップ）
├── bin/web.rs     bin: kpop-web（axum サーバー + JSON API）
├── lib.rs         TUI 版とブラウザ版で共有するライブラリ
├── db/            SQLite の操作・マイグレーション・CSV 入出力
├── models/        データ構造と config
├── scraper/       Genius / songbpm / ジャケットの取得
├── spotify/       お気に入りの取得（spotatui のトークンを利用）
└── tui/
    ├── app.rs     画面と状態
    ├── input.rs   キー入力の処理
    └── ui.rs      描画
web/               ブラウザ版のフロントエンド（バイナリに埋め込み）
scripts/           ブラウザ版の起動、データのバックアップ
```
