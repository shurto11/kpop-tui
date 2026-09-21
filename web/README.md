# kpop-web — ブラウザ版

TUI版 (`kpop-tui`) と同じ `kpop.db` を**読み取り専用**で開き、スマホのブラウザから
閲覧・検索できるようにしたもの。Tailscale 経由で tailnet 内にだけ公開する。

## 構成

```
src/lib.rs        db / models / scraper / spotify / tui を共有ライブラリ化
src/main.rs       bin: kpop-tui  (従来のTUI)
src/bin/web.rs    bin: kpop-web  (axum サーバー + JSON API)
web/index.html    フロントエンド（バイナリに include_str! で埋め込み）
web/app.css
web/app.js
scripts/kpop-web.sh  起動 + tailscale serve
```

DBは `Database::open_readonly()` で開くのでマイグレーションが走らず、
TUI版を起動したままでも書き込みが競合しない。

## 起動

```bash
# 初回のみ tailscale を sudo 無しで操作できるようにしておくと楽
sudo tailscale set --operator=$USER

./scripts/kpop-web.sh          # 既定 127.0.0.1:8787 + tailscale serve
./scripts/kpop-web.sh 9000     # ポート指定
./scripts/kpop-web.sh --stop   # tailnet への公開を解除
```

公開後は tailnet 内の端末（スマホなど）から:

```
https://ubuntubook.tail03bd8e.ts.net/
```

`tailscale serve` は tailnet 内限定で、インターネットには出ない
（出す場合は `tailscale funnel` だが個人データなので非推奨）。

サーバー単体で動かす場合:

```bash
cargo run --release --bin kpop-web -- --addr 127.0.0.1:8787
KPOP_WEB_ADDR=100.119.188.59:8787 cargo run --release --bin kpop-web
```

## 画面

| ルート | 内容 |
| --- | --- |
| `#/` | Today's Drops（TUIのMainMenuと同じ選曲ロジック）+ TrackData + Around-The-Day Drops + Credits |
| `#/search/writer` | Writer検索（前方一致サジェスト） |
| `#/search/track` | 曲検索（Artist → Track のサジェスト） |
| `#/writer/<name>` | Writer検索結果（WriterData / Statistics+Rank / 年別グラフ / 曲リスト） |
| `#/song/<artist>/<track>` | 曲検索結果 |
| `#/view/log` | 入力順（新しい順） |
| `#/view/credits` | CreditData（Artist順・日付順） |
| `#/view/tracks` `#/view/tracks/soty` `#/view/tracks/aoty` | TrackData |
| `#/view/artists` / `#/view/writers` | ArtistData / WriterData |

各Viewには絞り込み入力があり、9,000件超のCreditDataもスクロールに応じて
120行ずつ追加描画する。Artist/Track/Name のセルはリンクになっていて相互に辿れる。

## API

| エンドポイント | 返すもの |
| --- | --- |
| `GET /api/home` | 今日の曲の詳細 (`SongDetail`) |
| `GET /api/search/track?artist=&track=` | 曲詳細 |
| `GET /api/search/writer?name=` | Writer統計・順位・年別・曲リスト |
| `GET /api/view/{log,credits,tracks,artists,writers}` | 各テーブル（tracks は `?filter=soty\|aoty`） |
| `GET /api/suggest/{writers,artists,tracks}?q=&artist=` | 入力補完 |
| `GET /api/art?url=<spotify url>` | Spotify oEmbed のジャケットURL（結果をキャッシュ） |

## 未実装

書き込み系（Input / AutoAdd / Genius スクレイピング / Quiz）はTUI版のみ。
ブラウザ版は読み取り専用。
