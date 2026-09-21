'use strict';

/* ========== 小さなDOMヘルパー ========== */

function el(tag, props, ...kids) {
  const node = document.createElement(tag);
  if (props) {
    for (const [k, v] of Object.entries(props)) {
      if (v === null || v === undefined || v === false) continue;
      if (k === 'class') node.className = v;
      else if (k === 'text') node.textContent = v;
      else if (k === 'html') node.innerHTML = v;
      else if (k.startsWith('on')) node.addEventListener(k.slice(2), v);
      else node.setAttribute(k, v);
    }
  }
  for (const kid of kids.flat()) {
    if (kid === null || kid === undefined || kid === false) continue;
    node.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
  }
  return node;
}

const app = document.getElementById('app');

function setContent(...nodes) {
  app.replaceChildren(...nodes.flat().filter(Boolean));
}

function showLoading(msg) {
  setContent(el('div', { class: 'loading', text: msg || 'Loading…' }));
}

function showError(e) {
  setContent(el('div', { class: 'error', text: String(e && e.message ? e.message : e) }));
}

async function api(path, params) {
  const url = new URL(path, location.origin);
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      if (v !== undefined && v !== null && v !== '') url.searchParams.set(k, v);
    }
  }
  const res = await fetch(url);
  if (!res.ok) {
    let detail = res.statusText;
    try { detail = (await res.json()).error || detail; } catch (_) {}
    throw new Error(`${res.status}: ${detail}`);
  }
  return res.json();
}

/* ========== 登録済みWriter名（太字表示用） ========== */

let writerNamesPromise = null;

/** WriterDataに登録済みの名前のSet。セッション中1回だけ取得する */
function writerNames() {
  if (!writerNamesPromise) {
    writerNamesPromise = api('/api/writer-names').then((names) => new Set(names));
  }
  return writerNamesPromise;
}

/* ========== 表示ユーティリティ ========== */

const ROLE_ORDER = { lyricist: 1, composer: 2, arranger: 3, writer: 4 };

function roleClass(role) {
  const r = (role || '').toLowerCase();
  return ROLE_ORDER[r] ? `role-${r}` : '';
}

function fmtDuration(sec) {
  if (sec === null || sec === undefined) return '';
  return `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, '0')}`;
}

function fmtRelease(t) {
  if (t.is_title) return 'Title';
  if (t.is_prerelease) return 'Pre';
  return '-';
}

function fmtGenres(genres) {
  return genres && genres.length ? genres.map((g) => `#${g}`).join(' ') : '';
}

function calcAge(birth) {
  const m = /^(\d{4})-(\d{1,2})-(\d{1,2})$/.exec(birth || '');
  if (!m) return null;
  const [y, mo, d] = [+m[1], +m[2], +m[3]];
  const now = new Date();
  let age = now.getFullYear() - y;
  if (now.getMonth() + 1 < mo || (now.getMonth() + 1 === mo && now.getDate() < d)) age -= 1;
  return age;
}

function songLink(artist, track) {
  return `#/song/${encodeURIComponent(artist)}/${encodeURIComponent(track)}`;
}

function writerLink(name) {
  return `#/writer/${encodeURIComponent(name)}`;
}

function panel(title, count, body, extraClass) {
  const head = el('h2', { text: title });
  if (count !== null && count !== undefined) {
    head.append(' ', el('span', { class: 'count', text: `(${count})` }));
  }
  return el('section', { class: `panel ${extraClass || ''}` }, head, body);
}

/* ========== テーブル（大量行を段階的に描画） ========== */

const CHUNK = 120;

/**
 * cols:    [{label, cls}]
 * rows:    データ配列
 * cellsOf: (row) => [cell]  cell は文字列 or {text, cls, href}
 * textOf:  (row) => 絞り込み用の文字列（省略可）
 */
function dataTable(cols, rows, cellsOf, textOf) {
  const wrap = el('div', { class: 'table-wrap' });
  const tbody = el('tbody');
  const table = el(
    'table',
    null,
    el('thead', null, el('tr', null, cols.map((c) => el('th', { class: c.cls || '', text: c.label })))),
    tbody
  );
  wrap.append(table);

  const state = { rows, shown: 0 };
  const sentinel = el('div');

  function appendChunk() {
    const next = state.rows.slice(state.shown, state.shown + CHUNK);
    const frag = document.createDocumentFragment();
    for (const row of next) {
      const tr = el('tr');
      cellsOf(row).forEach((cell, i) => {
        const spec = typeof cell === 'object' && cell !== null ? cell : { text: cell };
        const cls = [cols[i].cls || '', spec.cls || ''].filter(Boolean).join(' ');
        const td = el('td', { class: cls });
        if (spec.href) td.append(el('a', { href: spec.href, class: 'link-row', text: spec.text ?? '' }));
        else td.textContent = spec.text ?? '';
        tr.append(td);
      });
      frag.append(tr);
    }
    tbody.append(frag);
    state.shown += next.length;
    if (state.shown >= state.rows.length) sentinel.remove();
    else wrap.append(sentinel);
  }

  const io = new IntersectionObserver((entries) => {
    if (entries.some((e) => e.isIntersecting)) appendChunk();
  }, { rootMargin: '400px' });
  io.observe(sentinel);

  appendChunk();

  // 絞り込み時に中身を差し替える
  wrap.setRows = (newRows) => {
    state.rows = newRows;
    state.shown = 0;
    tbody.replaceChildren();
    appendChunk();
  };
  wrap.textOf = textOf;
  return wrap;
}

/** 絞り込み入力 + テーブル をまとめたビュー */
function filterableTable(opts) {
  const { title, cols, rows, cellsOf, textOf, chips } = opts;
  const countEl = el('span', { class: 'count', text: `(${rows.length})` });
  const table = dataTable(cols, rows, cellsOf, textOf);

  const input = el('input', {
    type: 'search',
    placeholder: 'filter…',
    oninput: () => {
      const q = input.value.trim().toLowerCase();
      const filtered = q ? rows.filter((r) => textOf(r).toLowerCase().includes(q)) : rows;
      table.setRows(filtered);
      countEl.textContent = `(${filtered.length}${q ? ` / ${rows.length}` : ''})`;
    },
  });

  const toolbar = el('div', { class: 'toolbar' }, input, chips || null);
  const head = el('h2', { text: title });
  head.append(' ', countEl);

  return [toolbar, el('section', { class: 'panel' }, head, table)];
}

/* ========== 曲詳細（Home / 曲検索 共通） ========== */

function trackDataPanel(d) {
  const td = d.track_data;
  const credit = d.credits[0] || {};
  if (!td && !d.credits.length) {
    return panel('TrackData', null, el('div', { class: 'empty', text: 'No data' }));
  }

  const dl = el('dl', { class: 'kv' });
  const add = (label, value, cls) => {
    if (value === '' || value === null || value === undefined) return;
    dl.append(el('dt', { text: label }), el('dd', { class: cls || '', text: value }));
  };

  add('Album', credit.album || '', td && td.is_aoty ? 'gold' : '');
  add('Label', d.artist_label || '');
  add('Date', credit.date || '');
  if (td) {
    add('Genre', fmtGenres(td.genres), 'genre');
    add('Duration', fmtDuration(td.duration));
    add('BPM', td.bpm || '');
    add('Release', fmtRelease(td));
  }

  const flags = el('dd', null,
    el('span', { class: `badge ${td && td.is_aoty ? 'on' : ''}`, text: 'AOTY' }),
    ' ',
    el('span', { class: `badge ${td && td.is_soty ? 'on' : ''}`, text: 'SOTY' })
  );
  dl.append(el('dt', { text: 'Award' }), flags);

  const body = el('div', { class: 'panel-body' }, dl);
  if (td && td.spotify) {
    body.append(
      el('a', { class: 'spotify-link', href: td.spotify, target: '_blank', rel: 'noopener' }, '▶ Spotifyで開く')
    );
  }
  return panel('TrackData', null, body);
}

function artPanel(d) {
  const inner = d.art_url
    ? el('img', { class: 'art', src: d.art_url, alt: `${d.artist} - ${d.track}`, loading: 'lazy' })
    : el('div', { class: 'art-placeholder', text: '♪' });
  return panel('Art', null, el('div', { class: 'panel-body' }, inner));
}

function dropsPanel(d) {
  const box = el('div', { class: 'panel-body drops' });
  if (!d.around_day.length) {
    return panel('Around-The-Day Drops', null, el('div', { class: 'empty', text: 'No data' }));
  }
  for (const group of d.around_day) {
    box.append(el('div', { class: 'md', text: group.md }));
    if (!group.items.length) {
      box.append(el('ul', null, el('li', { class: 'faint', text: '(none)' })));
      continue;
    }
    const ul = el('ul');
    for (const it of group.items) {
      ul.append(
        el('li', null,
          el('span', { class: 'year', text: it.year }),
          el('a', { href: songLink(it.artist, it.track), class: 'link-row', text: `${it.artist} - ${it.track}` })
        )
      );
    }
    box.append(ul);
  }
  return panel('Around-The-Day Drops', null, box);
}

function creditsPanel(d) {
  if (!d.credits.length) {
    return panel('Credits', 0, el('div', { class: 'empty', text: 'クレジットが登録されていません' }));
  }
  const known = new Set(d.known_writers);
  const table = dataTable(
    [{ label: 'Role', cls: 'nowrap' }, { label: 'Name' }, { label: 'Count', cls: 'num nowrap' }],
    d.credits,
    (c) => [
      { text: c.role || '', cls: roleClass(c.role) },
      { text: c.name || '', href: c.name ? writerLink(c.name) : null, cls: known.has(c.name) ? 'known' : '' },
      { text: c.count === null || c.count === undefined ? '' : String(c.count) },
    ]
  );
  return panel('Credits', d.credits.length, table);
}

function songHead(d, labelText) {
  const soty = d.track_data && d.track_data.is_soty;
  return el('div', { class: 'song-head' },
    labelText ? el('span', { class: 'label', text: labelText }) : null,
    el('span', { class: 'artist', text: d.artist }),
    el('span', { class: 'sep', text: '—' }),
    el('span', { class: `track ${soty ? 'soty' : ''}`, text: d.track })
  );
}

function renderSongDetail(d, labelText) {
  if (!d.found) {
    setContent(
      songHead(d, labelText),
      el('div', { class: 'empty', text: 'データが見つかりませんでした' })
    );
    return;
  }
  setContent(
    songHead(d, labelText),
    el('div', { class: 'grid grid-3' }, artPanel(d), trackDataPanel(d), dropsPanel(d)),
    creditsPanel(d)
  );
}

/* ========== 画面: Home ========== */

async function screenHome() {
  showLoading();
  const d = await api('/api/home');
  if (!d.artist) {
    setContent(el('div', { class: 'empty', text: 'データがありません' }));
    return;
  }
  renderSongDetail(d, d.is_random_fallback ? 'New Track' : "Today's Drops");
}

/* ========== 画面: 曲検索 ========== */

async function screenSearchTrack() {
  const artistInput = el('input', { type: 'text', placeholder: 'artist…', autocapitalize: 'off', autocomplete: 'off' });
  const trackInput = el('input', { type: 'text', placeholder: 'track…', autocapitalize: 'off', autocomplete: 'off' });
  const artistList = el('ul', { class: 'suggests' });
  const trackList = el('ul', { class: 'suggests' });

  const fillSuggests = (list, items, onPick) => {
    list.replaceChildren(
      ...items.map((s) => el('li', null, el('button', { type: 'button', text: s, onclick: () => onPick(s) })))
    );
  };

  const refreshArtists = debounce(async () => {
    const items = await api('/api/suggest/artists', { q: artistInput.value.trim() });
    fillSuggests(artistList, items, (name) => {
      artistInput.value = name;
      artistList.replaceChildren();
      refreshTracks();
      trackInput.focus();
    });
  }, 150);

  const refreshTracks = debounce(async () => {
    const artist = artistInput.value.trim();
    if (!artist) { trackList.replaceChildren(); return; }
    const items = await api('/api/suggest/tracks', { artist, q: trackInput.value.trim() });
    fillSuggests(trackList, items, (track) => {
      location.hash = songLink(artist, track);
    });
  }, 150);

  artistInput.addEventListener('input', refreshArtists);
  trackInput.addEventListener('input', refreshTracks);
  trackInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && artistInput.value.trim() && trackInput.value.trim()) {
      location.hash = songLink(artistInput.value.trim(), trackInput.value.trim());
    }
  });

  setContent(
    searchModes('track'),
    el('section', { class: 'panel' },
      el('h2', { text: 'Song Search' }),
      el('div', { class: 'panel-body' },
        el('div', { class: 'field' }, el('label', { text: 'Artist' }), artistInput, artistList),
        el('div', { class: 'field' }, el('label', { text: 'Track' }), trackInput, trackList)
      )
    )
  );
  artistInput.focus();
  refreshArtists();
}

/* ========== 画面: Writer検索 ========== */

async function screenSearchWriter() {
  const input = el('input', { type: 'text', placeholder: 'writer name…', autocapitalize: 'off', autocomplete: 'off' });
  const list = el('ul', { class: 'suggests' });

  const refresh = debounce(async () => {
    const items = await api('/api/suggest/writers', { q: input.value.trim() });
    list.replaceChildren(
      ...items.map((s) =>
        el('li', null, el('button', { type: 'button', text: s, onclick: () => { location.hash = writerLink(s); } }))
      )
    );
  }, 150);

  input.addEventListener('input', refresh);
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && input.value.trim()) location.hash = writerLink(input.value.trim());
  });

  setContent(
    searchModes('writer'),
    el('section', { class: 'panel' },
      el('h2', { text: 'Writer Search' }),
      el('div', { class: 'panel-body' }, el('div', { class: 'field' }, input, list))
    )
  );
  input.focus();
  refresh();
}

function searchModes(active) {
  return el('nav', { class: 'search-modes' },
    el('a', { href: '#/search/writer', class: active === 'writer' ? 'active' : '', text: 'Writer' }),
    el('a', { href: '#/search/track', class: active === 'track' ? 'active' : '', text: 'Song' })
  );
}

/* ========== 画面: Writer検索結果 ========== */

async function screenWriterResult(name) {
  showLoading();
  const d = await api('/api/search/writer', { name });

  if (!d.found) {
    setContent(
      searchModes('writer'),
      el('div', { class: 'empty', text: `"${name}" は見つかりませんでした` })
    );
    return;
  }

  // Writer Data
  const dl = el('dl', { class: 'kv' });
  const add = (k, v) => { if (v) dl.append(el('dt', { text: k }), el('dd', { text: v })); };
  add('Name', d.name);
  if (d.aliases.length) add('AKA', d.aliases.join(', '));
  const wd = d.writer_data;
  if (wd) {
    add('Real Name', wd.real_name);
    if (wd.birth_date) {
      const age = calcAge(wd.birth_date);
      add('Birth Date', age === null ? wd.birth_date : `${wd.birth_date} (${age})`);
    }
    add('Birth Place', wd.birth_place);
    add('Occupation', wd.occupation);
    add('Agency', wd.agency);
    add('Debut', wd.debut);
    add('Memo', wd.memo);
  }
  const writerPanel = panel(
    'Writer Data',
    null,
    el('div', { class: 'panel-body' }, wd || d.aliases.length ? dl : el('span', { class: 'faint', text: 'No writer data registered' }))
  );

  // Statistics
  const statTable = el('table', null,
    el('thead', null, el('tr', null,
      el('th', { text: 'Role' }), el('th', { class: 'num', text: 'Sum' }), el('th', { class: 'num', text: 'Rank' })
    )),
    el('tbody', null, d.stats.map((s) =>
      el('tr', null,
        el('td', { class: roleClass(s.role), text: s.role }),
        el('td', { class: 'num', text: String(s.count) }),
        el('td', { class: 'num', text: s.count > 0 ? String(s.rank) : '-' })
      )
    ))
  );
  const statsPanel = panel('Statistics', null, el('div', { class: 'table-wrap' }, statTable));

  // Yearly（TUIと同じく 2022–2026）
  const yearMap = new Map(d.yearly.map((y) => [y.year, y.count]));
  const years = [];
  for (let y = 2022; y <= 2026; y++) years.push(String(y));
  const max = Math.max(1, ...years.map((y) => yearMap.get(y) || 0));
  const bars = el('div', { class: 'bars' },
    years.map((y) => {
      const c = yearMap.get(y) || 0;
      return el('div', { class: 'bar-col' },
        el('span', { class: 'bar-val', text: String(c) }),
        el('div', { class: 'bar', style: `height:${(c / max) * 100}%` }),
        el('span', { class: 'bar-label', text: `'${y.slice(2)}` })
      );
    })
  );
  const yearlyPanel = panel('Yearly', null, el('div', { class: 'panel-body' }, bars));

  // 曲リスト
  const sorted = [...d.songs];
  const songsTable = dataTable(
    [{ label: 'Artist' }, { label: 'Track' }, { label: 'Role', cls: 'nowrap' }, { label: 'Date', cls: 'nowrap col-mid' }],
    sorted,
    (s) => [
      { text: s.artist, href: songLink(s.artist, s.track) },
      { text: s.track, cls: s.is_soty ? 'gold' : '', href: songLink(s.artist, s.track) },
      { text: s.role || '', cls: roleClass(s.role) },
      { text: s.date || '' },
    ]
  );

  setContent(
    el('div', { class: 'song-head' },
      el('span', { class: 'label', text: 'Writer' }),
      el('span', { class: 'artist', text: d.name })
    ),
    el('div', { class: 'grid grid-3' }, writerPanel, statsPanel, yearlyPanel),
    panel('Songs', d.songs.length, songsTable)
  );
}

/* ========== 画面: View ========== */

const VIEW_TABS = [
  ['log', 'Log'],
  ['credits', 'CreditData'],
  ['tracks', 'TrackData'],
  ['artists', 'ArtistData'],
  ['writers', 'WriterData'],
];

function viewTabs(active) {
  return el('nav', { class: 'chips' },
    VIEW_TABS.map(([key, label]) =>
      el('a', { href: `#/view/${key}`, class: active === key ? 'active' : '', text: label })
    )
  );
}

async function screenView(kind, filter) {
  showLoading();

  if (kind === 'log' || kind === 'credits') {
    const [rows, known] = await Promise.all([
      api(kind === 'log' ? '/api/view/log' : '/api/view/credits'),
      writerNames(),
    ]);
    setContent(
      viewTabs(kind),
      filterableTable({
        title: kind === 'log' ? 'Log (新しい順)' : 'CreditData',
        cols: [
          { label: 'Artist' },
          { label: 'Label', cls: 'col-lo' },
          { label: 'Date', cls: 'nowrap col-mid' },
          { label: 'Album', cls: 'col-lo' },
          { label: 'Track' },
          { label: 'Role', cls: 'nowrap' },
          { label: 'Name' },
          { label: 'Cnt', cls: 'num col-lo' },
        ],
        rows,
        cellsOf: (s) => [
          { text: s.artist, href: songLink(s.artist, s.track) },
          s.label || '',
          s.date || '',
          { text: s.album || '', cls: s.is_aoty ? 'gold' : '' },
          { text: s.track, cls: s.is_soty ? 'gold' : '', href: songLink(s.artist, s.track) },
          { text: s.role || '', cls: roleClass(s.role) },
          { text: s.name || '', href: s.name ? writerLink(s.name) : null, cls: known.has(s.name) ? 'known' : '' },
          s.count === null || s.count === undefined ? '' : String(s.count),
        ],
        textOf: (s) => [s.artist, s.label, s.date, s.album, s.track, s.role, s.name].join(' '),
      })
    );
    return;
  }

  if (kind === 'tracks') {
    const f = filter || 'all';
    const rows = await api('/api/view/tracks', { filter: f });
    const chips = el('div', { class: 'chips' },
      [['all', 'All'], ['soty', 'SOTY'], ['aoty', 'AOTY']].map(([key, label]) =>
        el('a', {
          href: key === 'all' ? '#/view/tracks' : `#/view/tracks/${key}`,
          class: f === key ? 'active' : '',
          text: label,
        })
      )
    );
    setContent(
      viewTabs('tracks'),
      filterableTable({
        title: f === 'all' ? 'TrackData' : `TrackData [${f.toUpperCase()}]`,
        cols: [
          { label: 'Artist' },
          { label: 'Label', cls: 'col-lo' },
          { label: 'Date', cls: 'nowrap col-mid' },
          { label: 'Album', cls: 'col-lo' },
          { label: 'Track' },
          { label: 'Genre', cls: 'col-lo' },
          { label: 'Dur', cls: 'num nowrap' },
          { label: 'BPM', cls: 'num col-mid' },
          { label: 'Rel', cls: 'nowrap col-lo' },
          { label: 'A', cls: 'col-mid' },
          { label: 'S', cls: 'col-mid' },
        ],
        rows,
        cellsOf: (t) => [
          { text: t.artist, href: songLink(t.artist, t.track) },
          t.label || '',
          t.date || '',
          { text: t.album || '', cls: t.is_aoty ? 'gold' : '' },
          { text: t.track, cls: t.is_soty ? 'gold' : '', href: songLink(t.artist, t.track) },
          { text: fmtGenres(t.genres), cls: 'genre' },
          fmtDuration(t.duration),
          t.bpm || '',
          fmtRelease(t),
          { text: t.is_aoty ? '★' : '', cls: 'gold' },
          { text: t.is_soty ? '★' : '', cls: 'gold' },
        ],
        textOf: (t) => [t.artist, t.label, t.date, t.album, t.track, fmtGenres(t.genres), t.bpm].join(' '),
        chips,
      })
    );
    return;
  }

  if (kind === 'artists') {
    const rows = await api('/api/view/artists');
    setContent(
      viewTabs('artists'),
      filterableTable({
        title: 'ArtistData',
        cols: [{ label: 'Artist' }, { label: 'Label' }, { label: 'Memo', cls: 'col-mid' }],
        rows,
        cellsOf: (a) => [a.artist, a.label || '', a.memo || ''],
        textOf: (a) => [a.artist, a.label, a.memo].join(' '),
      })
    );
    return;
  }

  if (kind === 'writers') {
    const rows = await api('/api/view/writers');
    setContent(
      viewTabs('writers'),
      filterableTable({
        title: 'WriterData',
        cols: [
          { label: 'Name' },
          { label: 'RealName', cls: 'col-mid' },
          { label: 'BirthDate', cls: 'nowrap col-lo' },
          { label: 'BirthPlace', cls: 'col-lo' },
          { label: 'Occupation', cls: 'col-lo' },
          { label: 'Agency', cls: 'col-lo' },
          { label: 'Debut', cls: 'col-lo' },
          { label: 'Memo', cls: 'col-lo' },
        ],
        rows,
        cellsOf: (w) => [
          { text: w.name, href: writerLink(w.name) },
          w.real_name || '',
          w.birth_date || '',
          w.birth_place || '',
          w.occupation || '',
          w.agency || '',
          w.debut || '',
          w.memo || '',
        ],
        textOf: (w) => [w.name, w.real_name, w.birth_place, w.occupation, w.agency, w.memo].join(' '),
      })
    );
    return;
  }

  setContent(viewTabs(''), el('div', { class: 'empty', text: 'Unknown view' }));
}

/* ========== ルーティング ========== */

function debounce(fn, ms) {
  let t;
  return (...args) => {
    clearTimeout(t);
    t = setTimeout(() => fn(...args), ms);
  };
}

function markTab(name) {
  document.querySelectorAll('.tabs a').forEach((a) => {
    a.classList.toggle('active', a.dataset.tab === name);
  });
}

async function route() {
  const hash = location.hash.replace(/^#/, '') || '/';
  const parts = hash.split('/').filter(Boolean).map(decodeURIComponent);

  try {
    if (parts.length === 0) {
      markTab('home');
      await screenHome();
    } else if (parts[0] === 'search') {
      markTab('search');
      if (parts[1] === 'track') await screenSearchTrack();
      else await screenSearchWriter();
    } else if (parts[0] === 'writer') {
      markTab('search');
      await screenWriterResult(parts[1] || '');
    } else if (parts[0] === 'song') {
      markTab('search');
      showLoading();
      const d = await api('/api/search/track', { artist: parts[1] || '', track: parts[2] || '' });
      renderSongDetail(d, null);
    } else if (parts[0] === 'view') {
      markTab('view');
      await screenView(parts[1] || 'log', parts[2]);
    } else {
      markTab('home');
      await screenHome();
    }
    window.scrollTo(0, 0);
  } catch (e) {
    showError(e);
  }
}

window.addEventListener('hashchange', route);
route();
