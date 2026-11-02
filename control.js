// The control centre (feature rust-core; settings: control, control_layout): in fullscreen the HUD gets a window of its
// own, control.html, while the game's window keeps the world and what the layout leaves on it. Both pages are hud.html's
// (scripts/hud.mjs puts this ahead of hud.js in each); the app sends both every planet message, and each page shows
// only its share of the toasts and cues. Layouts: 1 the drawer's tabs, 2 a sidebar with everything, 3 wide with a live
// planet, 4 a stream deck, 5 mission control. test/control.test.cjs tests the parts without a page.
const CONTROL = (() => {
  const LAYOUTS = [1, 2, 3, 4, 5];
  const pick = n => (LAYOUTS.includes(+n) ? +n : 1);
  // the cues that want an answer (the dragon, the transmission picker, zeno, a ki tree gift): the panel's, where the
  // buttons are. Sounds, flashes, the shake and notifications stay with the game; an achievement recounts on both
  const PANEL_CUES = ['wish', 'itxPick', 'wishDone', 'zeno', 'treeGift'], BOTH = ['achieved'];
  // the game screen keeps the toasts, except in layout 2, which leaves it nothing
  const toastsOnPanel = layout => pick(layout) === 2;

  // which page this is: the control window ('panel'), the game's while it has one ('game'), or the widget as ever ('solo')
  const role = (control, s) => (control ? 'panel' : s && s.control_on ? 'game' : 'solo');

  // whether this page shows a toast ('toast') or a cue (its ui call's name)
  function shows(who, layout, what) {
    if (who === 'solo') return true;
    const panel = who === 'panel';
    if (what === 'toast') return panel === toastsOnPanel(layout);
    if (BOTH.includes(what)) return true;
    return panel === PANEL_CUES.includes(what);
  }

  // the settings as each page takes them: the panel is the drawer skin, always awake, in a plain window; the game keeps
  // the drawer's power chip (1, and 4's scoreboard) or the classic dock (5), and shows it always
  function lens(s, who) {
    if (!s || who === 'solo') return s;
    const layout = pick(s.control_layout);
    if (who === 'panel') return Object.assign({}, s, { skin: 'drawer', layout: 7, hud: 'always', mode: 'window', opacity: 1, clouds: 'never', ui_size: 'medium' });
    return Object.assign({}, s, layout === 5 ? { skin: 'classic', layout: 7 } : { skin: 'drawer' }, { hud: 'always' });
  }

  // what each layout's panel holds, as build() lays it out: its columns (a width in px, or the rest), each a list of
  // parts: 'hud' the chip and round buttons, 'top' those as a strip, 'tabs:…' parts behind tabs, 'shop' 'tree' 'heroes'
  // 'ach' a part under its heading, 'live' the planet, 'deck' the tiles, 'toast' room for the panel's toasts; 'more' the
  // parts behind the deck's last tile. `shop` dresses the upgrades list, `tight` the ki tree in a column
  const PLAN = {
    1: { cols: [{ parts: ['hud', 'tabs:up,tree,heroes,ach'] }], shop: ['desc'] },
    2: { cols: [{ parts: ['top', 'shop', 'tree', 'heroes', 'toast'] }], shop: ['grid2', 'nobulk'], tight: true },
    3: { cols: [{ w: 280, parts: ['live', 'hud'] }, { parts: ['tabs:up,tree,heroes,ach'] }], shop: ['grid2'] },
    4: { cols: [{ parts: ['top', 'deck'] }], more: ['tree', 'heroes', 'ach'] },
    5: { cols: [{ w: 250, parts: ['hud', 'heroes', 'ach'] }, { w: 260, parts: ['shop'] }, { parts: ['tree'] }], tight: true },
  };
  // the parts build() can place, and the tabs it can show
  const PARTS = ['hud', 'top', 'shop', 'tree', 'heroes', 'ach', 'live', 'deck', 'toast'], TABS = ['up', 'tree', 'heroes', 'ach'];
  // every part a layout shows (tabs and the deck's sheet opened out), as build() places them
  const parts = layout => { const p = PLAN[pick(layout)]; return [...p.cols.flatMap(c => c.parts), ...(p.more || []).map(k => 'more:' + k)].flatMap(x => (x.startsWith('tabs:') ? x.slice(5).split(',') : [x])); };
  // whether the panel is being used: focused, or the pointer over it lately (hud.js fetches the menus more often then)
  let usedAt = -1e9;
  const active = () => typeof document === 'object' && (document.hasFocus() || performance.now() - usedAt < 5000);

  // a stream deck's tiles: the hero's boost (tall, first), every upgrade, then transmission and the tile for the rest
  function deck(rows, itx) {
    return [{ kind: 'kaio' }, ...rows.map(r => ({ kind: 'buy', ...r })), ...(itx ? [{ kind: 'itx' }] : []), { kind: 'more' }];
  }

  // a boost button's name as the page shows it: its title (the game's per-hero name) or its text without the countdown
  const label = (title, text) => (title || String(text || '').replace(/\s*\d+(:\d+)?s?$/, '')).trim();

  // the counts on the tabs: upgrades you can buy, ki tree nodes you can learn, heroes and achievements had of all
  function counts(menu, shopCan) {
    const m = menu || {}, chars = m.chars || [], ach = m.ach || [];
    return { up: shopCan || '', tree: (m.tree || []).filter(n => n && n.can).length || '', heroes: chars.length ? `${chars.filter(c => c && c.open).length}/${chars.length}` : '', ach: ach.length ? `${ach.filter(a => a && a.done).length}/${ach.length}` : '' };
  }

  // the game's numbers: big ones arrive as text already
  const num = v => (typeof v === 'number' ? v.toLocaleString('en-US') : String(v));

  const CSS = `
/* the game's window while the control centre has the HUD: the world, and only what the layout leaves on it */
html.cc .w > :not(.dw-chip):not(.w-dock), html.cc .dw, html.cc #shop { display: none !important; }
html.cc:not([data-cc="1"]):not([data-cc="4"]) .w .dw-chip, html.cc:not([data-cc="5"]) .w .w-dock { display: none !important; }
html.cc.dw-open .stage { transform: none !important; }
html.cc .toast { top: auto !important; }   /* a buff line (.fxs) would give it a top too, and stretch it down to its bottom */
html.cc[data-cc="1"] .w .dw-chip { left: 18px; top: 18px; transform: scale(1.35); transform-origin: 0 0; }
html.cc[data-cc="4"] .w .dw-chip { left: 50%; top: auto; bottom: 22px; align-items: center; padding: 10px 22px 12px; transform: translateX(-50%) scale(1.6); transform-origin: 50% 100%; }
html.cc[data-cc="5"] .w .w-dock { left: calc(50% - 230px); right: calc(50% - 230px); bottom: 14px; padding-right: 14px; transform: scale(1.25); transform-origin: 50% 100%; }
html.cc[data-cc="5"] .w .w-dock .w-btns { display: none !important; }

/* the control window */
html.control, html.control body { background: var(--cc-bg) !important; overflow: hidden; }
html.control { --cc-bg: #f4f3ef; --chip: #fff; --edge: rgba(0,0,0,.09); --shadow: 0 1px 3px rgba(0,0,0,.08); --ink: #1d1c1a; --muted: #6f6c66; --line: rgba(0,0,0,.1); --gold: #c98a12; --row: #f6f5f2; --can: #fff3d6; }
html.control[data-theme="dark"] { --cc-bg: #141419; --chip: rgba(24,24,30,.94); --edge: rgba(255,255,255,.12); --shadow: 0 2px 10px rgba(0,0,0,.32), 0 0 0 .5px rgba(0,0,0,.6); --ink: #f3f1ec; --muted: #a3a09a; --line: rgba(255,255,255,.14); --gold: #f2b53a; --row: rgba(255,255,255,.06); --can: rgba(242,181,58,.14); }
html.control .page > :not(#game):not(.menu) { display: none !important; }
/* the page (its boxes: the wish, ascending, the ki tree a gift opens) over the panel, taking clicks only on them */
html.control .scroller, html.control .page { z-index: 10; pointer-events: none; }
html.control .page .menu { pointer-events: auto; }
html.control #game { position: fixed; inset: 0; z-index: 30; pointer-events: none; background: none !important; border: 0; }
html.control #game > :not(.wish):not(.toast):not(.stage) { display: none !important; }
html.control #game > .stage { visibility: hidden; }   /* kept its size: the page sizes its (unused) canvas from it */
html.control #game .wish { pointer-events: auto; position: fixed; left: 50%; top: 50%; bottom: auto; right: auto; transform: translate(-50%, -50%); width: min(360px, calc(100% - 24px)); max-height: calc(100% - 24px); overflow: auto; padding: 14px; background: var(--cc-bg); border: 1px solid var(--edge); border-radius: 16px; box-shadow: 0 12px 40px rgba(0,0,0,.45); }
html.control #game .wish p { margin: 0 0 4px; font-size: 20px; color: var(--ink); -webkit-text-stroke: 0; text-shadow: none; }
html.control #game .wish button { color: var(--ink); background: var(--row); border-color: var(--line); }
html.control #game .wish button b { color: var(--ink); } html.control #game .wish button span { color: var(--muted); }
html.control #game .toast { position: fixed; top: auto; bottom: 14px; left: 50%; transform: translate(-50%, 6px); max-width: calc(100% - 32px); padding: 6px 14px 7px; font-size: 15px; font-style: normal; -webkit-text-stroke: 0; color: var(--ink); background: var(--chip); border: 1px solid var(--edge); border-radius: 999px; box-shadow: var(--shadow); }
html.control #game .toast.on { transform: translate(-50%, 0); }
#cc { --pad: 10px; position: fixed; inset: 0; z-index: 1; display: flex; gap: var(--pad); padding: var(--pad); box-sizing: border-box; color: var(--ink); font-size: 12px; overflow: hidden; }
#cc * { box-sizing: border-box; }
#cc .col { display: flex; flex-direction: column; gap: 8px; min-width: 0; min-height: 0; flex: 1; }
#cc .card { background: var(--chip); border: 1px solid var(--edge); border-radius: 14px; box-shadow: var(--shadow); }
#cc .grow { flex: 1 1 0; min-height: 0; display: flex; flex-direction: column; overflow: hidden; }
#cc .lbl { flex: none; display: flex; justify-content: space-between; padding: 0 2px; font-size: 9px; text-transform: uppercase; letter-spacing: .1em; color: var(--muted); }
#cc .lbl b { font-weight: 400; color: var(--gold); }
#cc .row { flex: none; display: flex; gap: 8px; align-items: center; }
/* the drawer's chip and buttons, out of their corners */
#cc .w { position: static; inset: auto; z-index: auto; pointer-events: auto; display: contents; }
#cc .w .dw-tab, #cc .w [data-w="close"], #cc .w [data-w="up"], #cc[data-l="4"] .w :is([data-w="kaio"], [data-w="itx"]) { display: none !important; }
#cc .dw-chip { position: static; flex: none; display: flex; padding: 12px 16px 13px; gap: 6px; }
#cc .dw-n { font-size: 38px; } #cc .dw-r { font-size: 11px; } #cc .dw-chip > i { height: 3px; }
#cc .dw-rail { position: static; flex: none; display: flex; flex-wrap: wrap; gap: 8px; }
#cc .dw-rail button { --b: 40px; }
#cc .dw-rail button:hover { background: var(--row); }
#cc .dw-rail [data-w="snd"][aria-pressed="false"] { color: var(--muted); }
/* tabs, as the drawer's header */
#cc .tabs { flex: none; display: flex; gap: 2px; padding: 4px; border-bottom: 1px solid var(--line); }
#cc .tabs button { flex: 1 1 auto; padding: 7px 4px; font: inherit; font-size: 10px; text-transform: uppercase; letter-spacing: .02em; white-space: nowrap; color: var(--muted); background: none; border: 0; border-radius: 8px; cursor: pointer; }
#cc .tabs button:hover { color: var(--ink); }
#cc .tabs button.on { color: var(--ink); background: var(--row); } #cc .tabs b { font-weight: 400; color: var(--gold); }
#cc .pane { flex: 1 1 0; min-height: 0; overflow: hidden auto; scrollbar-width: thin; }
/* the page's own upgrades list */
#cc #shop { position: static; inset: auto; transform: none; width: auto; min-width: 0; max-height: none; display: flex; flex-direction: column; flex-wrap: nowrap; justify-content: flex-start; align-content: start; gap: 3px; padding: 6px; margin: 0; overflow: visible; pointer-events: auto; background: none; border: 0; border-radius: 0; box-shadow: none; visibility: visible; opacity: 1; }
#cc #shop .bulk { display: flex; justify-content: flex-start; gap: 4px; padding: 0 0 4px; }
#cc #shop .bulk button[data-n] { width: auto; min-width: 0; padding: 2px 12px; border: 0; border-radius: 999px; font-size: 10px; flex: none; display: inline-block; background: var(--row); }
#cc #shop .bulk button[aria-pressed="true"] { background: var(--can); box-shadow: inset 0 0 0 1px var(--gold); }
#cc #shop .goal { display: block; padding: 2px 2px 6px; background: none; color: var(--muted); font-size: 10px; }
#cc #shop .goal i { display: block; height: 2px; margin-top: 4px; background: var(--line); }
#cc #shop button[data-buy] { position: relative; width: 100%; min-width: 0; display: flex; flex-direction: row; flex-wrap: wrap; align-items: baseline; justify-content: space-between; gap: 0 8px; padding: 6px 9px; line-height: 1.3; color: var(--ink); background: var(--row); border: 0; border-radius: 9px; box-shadow: none; opacity: 1; }
#cc #shop button[data-buy]:not(:disabled) { background: var(--can); }
#cc #shop button[data-buy]:disabled { opacity: .5; }
#cc #shop b { flex: 1 1 0; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-transform: none; font-size: 12px; }
#cc #shop em { flex: none; font-size: 12px; }
#cc #shop button span { display: none; position: static; width: auto; max-width: none; padding: 0; border: 0; box-shadow: none; background: none; transform: none; }
#cc #shop.desc button span { display: block; flex-basis: 100%; font-size: 10px; color: var(--muted); }
#cc #shop.grid2 { display: grid; grid-template-columns: 1fr 1fr; }
#cc #shop.grid2 .bulk, #cc #shop.grid2 .goal { grid-column: 1 / -1; }
#cc #shop.grid2 button[data-buy] { flex-direction: column; align-items: start; flex-wrap: nowrap; }
#cc #shop.grid2 b { width: 100%; flex: none; }
#cc #shop.nobulk .bulk { display: none; }
#cc .tip { display: none !important; }
/* the ki tree, heroes and achievements: drawn here from the game's menus, dressed as the drawer dresses them */
#cc .menu-body { position: static; overflow: visible; padding: 6px; max-height: none; display: grid; gap: 4px; align-content: start; font-size: 11px; }
#cc .menu-body.tree { grid-template-columns: repeat(3, minmax(0, 1fr)); }
#cc .menu-body.angels { grid-template-columns: repeat(2, minmax(0, 1fr)); }
#cc .node, #cc .char, #cc .tree-head button { color: var(--ink); background: var(--row); border-color: var(--line); }
#cc .node { padding: 6px 8px; border-radius: 10px; gap: 2px; min-width: 0; } #cc .node.can { border-color: var(--gold); background: var(--can); }
#cc .node b, #cc .node em { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 100%; }
#cc .node span { font-size: 10px; color: var(--muted); }
#cc .node.locked { opacity: .55; }
#cc .tight .node span { display: none; } #cc .tight .node { padding: 4px 7px; } #cc .tight .node i:empty { display: none; }
#cc .one.tree { grid-template-columns: 1fr; } #cc .one .node { grid-row: auto !important; grid-column: auto !important; }
#cc .menu-body.chars { grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 6px; }
#cc .char { padding: 8px 4px 6px; border-radius: 12px; } #cc .char.on { border-color: var(--gold); background: var(--can); }
#cc .char canvas { width: 52px; image-rendering: pixelated; }
#cc .carousel { display: flex; gap: 6px; padding: 6px; overflow: auto hidden; }
#cc .carousel .char { flex: 1 0 0; min-width: 0; padding: 6px 2px 4px; gap: 3px; font-size: 10px; }
#cc .carousel .char span { display: none; } #cc .carousel .char canvas { width: 34px; }
#cc [data-icons="pills"] { display: flex; flex-wrap: wrap; gap: 4px; }
#cc .ach { color: var(--muted); background: var(--row); border-color: var(--line); padding: 4px 10px; font-size: 10px; }
#cc .ach.done { color: var(--ink); background: var(--can); border-color: var(--gold); }
#cc .ach span { display: none; } #cc .ach.open span { display: inline; }
#cc .tree-head { grid-column: 1 / -1; margin: 0 2px 2px; font-size: 10px; }
#cc .tree-head button + button { margin-left: 4px; }
/* 3: the planet as the game's window shows it, a few frames a second */
#cc .live { position: relative; flex: none; height: 150px; overflow: hidden; border-radius: 12px; border: 1px solid var(--edge); background: #000; }
#cc .live canvas { display: block; width: 100%; height: 100%; object-fit: contain; image-rendering: pixelated; }
#cc .live em { position: absolute; left: 8px; top: 8px; font-style: normal; font-size: 9px; text-transform: uppercase; letter-spacing: .08em; color: #fff; background: rgba(224,51,43,.9); padding: 2px 7px; border-radius: 999px; }
/* 4: the stream deck */
#cc .deck { flex: 1; min-height: 0; display: grid; grid-auto-flow: column; grid-template-rows: repeat(2, minmax(0, 1fr)); grid-auto-columns: minmax(0, 1fr); gap: 8px; }
#cc .tile { position: relative; display: flex; flex-direction: column; justify-content: space-between; min-height: 0; padding: 10px 11px; overflow: hidden; font: inherit; text-align: left; color: var(--ink); background: var(--row); border: 1px solid var(--line); border-radius: 14px; cursor: pointer; }
#cc .tile:disabled { cursor: default; }
#cc .tile.can { background: var(--can); border-color: var(--gold); box-shadow: 0 0 12px rgba(242,181,58,.15); }
#cc .tile.no { opacity: .5; }
#cc .tile b { display: block; font-weight: 400; font-size: 12px; line-height: 1.2; }
#cc .tile.big span { display: block; max-width: 100%; text-align: center; }
#cc .tile i { font-style: normal; color: var(--gold); font-size: 10px; }
#cc .tile em { font-style: normal; font-size: 19px; font-family: var(--serif); color: var(--gold); letter-spacing: -.01em; white-space: nowrap; }
#cc .tile.no em { color: var(--muted); }
#cc .tile.big { grid-row: span 2; align-items: center; justify-content: center; gap: 8px; text-transform: uppercase; font-size: 11px; letter-spacing: .08em; color: #fff; border: 0; }
#cc .tile.big svg { width: 34px; height: 34px; }
#cc .tile.t-kaio { background: linear-gradient(to top, var(--kc, #e0332b) var(--f, 100%), var(--kp, rgba(224,51,43,.35)) var(--f, 100%)); }
#cc .tile.t-itx { grid-row: auto; background: linear-gradient(to top, var(--kc, #1f3a57) var(--f, 100%), var(--kp, rgba(31,58,87,.45)) var(--f, 100%)); color: #cfe0f2; font-size: 10px; }
#cc .tile.t-itx svg { width: 26px; height: 26px; }
#cc .tile.big:disabled { opacity: .8; }
#cc .tile.more { align-items: center; justify-content: center; gap: 4px; color: var(--muted); text-transform: uppercase; font-size: 10px; letter-spacing: .06em; text-align: center; }
#cc .tile.more b { color: var(--gold); font-size: 18px; font-family: var(--serif); }
#cc .tile.more span { font-size: 9px; line-height: 1.5; }
#cc .toastslot { flex: none; height: 34px; }
#cc .bulkt { flex: none; display: flex; flex-direction: column; align-items: center; justify-content: center; padding: 6px 14px; font: inherit; color: var(--ink); cursor: pointer; }
#cc .bulkt b { font-weight: 400; font-size: 16px; color: var(--gold); }
#cc .sheet { position: fixed; inset: var(--pad); z-index: 5; display: flex; flex-direction: column; background: var(--cc-bg); box-shadow: 0 12px 40px rgba(0,0,0,.45); }
#cc .sheet .x { flex: none; width: 34px; font: inherit; font-size: 16px; color: var(--muted); background: none; border: 0; cursor: pointer; }
/* 4 and 2: the chip as a strip beside the buttons */
#cc .top .dw-chip { flex: 1; min-width: 0; flex-direction: row; align-items: center; gap: 14px; }
#cc .top .dw-chip > i { flex: 0 0 90px; }
#cc[data-l="2"] .top .dw-chip { flex-direction: column; align-items: stretch; gap: 5px; } #cc[data-l="2"] .top .dw-chip > i { flex: none; }
#cc .dw-chip.sm .dw-n { font-size: 26px; }
html.control.photo #cc { visibility: hidden; }
`;

  const SVG = {
    more: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h.01M12 12h.01M19 12h.01" stroke="currentColor" stroke-width="3" stroke-linecap="round"/></svg>',
  };

  // the control window's page: the drawer's chip and buttons, the page's own upgrades list, and the ki tree, heroes and
  // achievements drawn here from the game's menus. api: { game (hud.js's), invoke, settings() (as the app sent them),
  // fresh() (when hud.js last fetched the menus) }
  function mount(api) {
    const { game } = api, root = document.documentElement, $ = id => document.getElementById(id);
    // an element with its class and children; strings go in as text (the game's names never as markup)
    const h = (tag, cls, ...kids) => { const e = document.createElement(tag); if (cls) e.className = cls; e.append(...kids.filter(k => typeof k === 'string' || k instanceof Node)); return e; };
    const button = (cls, ...kids) => { const b = h('button', cls, ...kids); b.type = 'button'; return b; };
    const cc = h('div'); cc.id = 'cc';
    let layout = 0, tab = 'up', sheet = false, menuAt = -1, live = null, angelsOn = false;
    const thumbs = {};
    const thumb = id => thumbs[id] || (thumbs[id] = game.thumb(id));
    // what a click changed comes back with the menus hud.js fetches again: drawn once it's in
    const redraw = () => { menuAt = -1; tick(); };
    const later = () => setTimeout(redraw, 120);

    // ── the menus, as the page's menu draws them
    const head = (lead, rest, ...btns) => h('p', 'tree-head', h('span', '', h('b', '', lead), rest), h('span', '', ...btns));
    const act = (text, fn) => { const b = button('', text); b.addEventListener('click', fn); return b; };
    function node(cls, col, row, top, name, what, foot, can, fn) {
      const b = button(cls, h('i', '', top), h('b', '', name), h('span', '', what), h('em', '', foot));
      b.disabled = !can; b.style.gridColumn = col + 1; b.style.gridRow = row + 2;
      b.addEventListener('click', fn);
      return b;
    }
    function tree(body, tight) {
      const m = game.menu();
      body.className = 'menu-body tree' + (tight ? ' tight' : ''); body.textContent = '';
      body.append(head(num(m.seeds), ` ki seeds${m.seeds || m.spent ? ` · ×${m.boost} on everything` : ' — ascend to earn some'}`,
        m.spent && act('reset', () => { game.respec(); later(); }), game.angels().gods && act('angels', () => { angelsOn = true; redraw(); })));
      (m.branches || []).forEach((br, col) => m.tree.filter(n => n.branch === br.name).forEach((n, row) => body.append(node(
        'node' + (n.can ? ' can' : '') + (n.lvl ? ' on' : '') + (n.open ? '' : ' locked') + (n.tier === 3 ? ' cap' : ''), col, row,
        row ? '' : `${br.name} · ${br.pts} pts`, n.name, n.what,
        !n.open ? `needs ${n.need} in ${n.branch}` : (n.max === Infinity || n.max === null ? 'lv ' + n.lvl : n.lvl + '/' + n.max) + (n.lvl < n.max ? ` · ${num(n.cost)} seed${n.cost > 1 ? 's' : ''}` : ''),
        n.can, () => { game.learn(n.id); later(); }))));
    }
    function angels(body) {
      const an = game.angels();
      body.className = 'menu-body tree angels'; body.textContent = '';
      body.append(head(num(an.godki), ` god ki · ×${an.boost} on everything`, an.auto && act('self-ascend: ' + an.auto, () => { game.autoNext(); later(); }), act('ki tree', () => { angelsOn = false; redraw(); })));
      ['whis', 'vados'].forEach((who, col) => an.list.filter(a => a.who === who).forEach((a, row) => body.append(node(
        'node' + (a.can ? ' can' : '') + (a.on ? ' on' : '') + (a.open ? '' : ' locked') + (a.tier === 2 ? ' cap' : ''), col, row,
        row ? '' : who, a.name, a.what, a.on ? 'learned' : !a.open ? `needs ${a.need} from ${who}` : a.cost + ' god ki',
        a.can, () => { game.learnAngel(a.id); later(); }))));
    }
    function heroes(body, row) {
      body.className = row ? 'carousel' : 'menu-body chars'; body.textContent = '';
      for (const c of game.menu().chars) {
        const b = button('char' + (c.on ? ' on' : '') + (c.open ? '' : ' locked'), thumb(c.id), h('b', '', c.name), h('span', '', c.on ? 'playing' : c.open ? 'tap to play' : 'unlock: ' + c.need));
        if (c.open) b.addEventListener('click', () => { game.pick(c.id); later(); });
        body.append(b);
      }
    }
    function ach(body) {
      body.className = 'menu-body'; body.dataset.icons = 'pills'; body.textContent = '';
      for (const a of game.menu().ach) {
        const d = button('ach' + (a.done ? ' done' : ''), h('b', '', a.name), h('span', '', a.text)); d.title = a.text;
        d.addEventListener('click', () => d.classList.toggle('open'));
        body.append(d);
      }
    }
    const MENUS = { tree: b => (angelsOn ? angels(b) : tree(b, b.dataset.tight === '1')), heroes: b => heroes(b, b.dataset.row === '1'), ach };

    // ── the parts
    const w = () => document.querySelector('.w');
    const shop = cls => { const s = $('shop'); s.classList.remove('desc', 'grid2', 'nobulk'); if (cls.length) s.classList.add(...cls); return s; };
    const menuPart = (kind, opts = {}) => { const b = h('div', 'menu-body'); b.dataset.menu = kind; if (opts.tight) b.dataset.tight = '1'; if (opts.row) b.dataset.row = '1'; return b; };
    const count = key => { const b = h('b'); b.dataset.n = key; return b; };
    const heading = (text, key) => h('div', 'lbl', h('span', '', text), count(key));
    const NAMES = { up: 'upgrades', tree: 'ki tree', heroes: 'heroes', ach: 'achievements' };
    function tabs(list, plan) {
      if (!list.includes(tab)) tab = list[0];
      const box = h('div', 'card grow'), bar = h('div', 'tabs'), pane = h('div', 'pane');
      for (const k of list) { const b = button(k === tab ? 'on' : '', NAMES[k] + ' ', count(k)); b.dataset.tab = k; bar.append(b); }
      bar.addEventListener('click', e => { const b = e.target.closest('[data-tab]'); if (b) { tab = b.dataset.tab; build(); } });
      pane.append(tab === 'up' ? shop(plan.shop || []) : menuPart(tab));
      box.append(bar, pane);
      return box;
    }
    function liveView() {
      const cv = h('canvas'), box = h('div', 'live', cv, h('em', '', 'live')), buf = h('canvas');
      let busy = false;
      const frame = () => {
        if (busy || document.hidden || !box.isConnected) return; busy = true;
        Promise.all([api.invoke('planet_spot').then(JSON.parse), api.invoke('planet_pixels')]).then(([s, px]) => {
          if (!px || px.byteLength !== s.w * s.h * 4) return;   // too soon, the app sends nothing (control.rs)
          buf.width = s.w; buf.height = s.h; buf.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(px), s.w, s.h), 0, 0);
          // the planet and some sky round it, at the box's shape
          const k = cv.clientWidth / Math.max(1, cv.clientHeight), side = Math.min(s.h, Math.max(s.r * 3.2, 40)), wd = Math.min(s.w, side * k);
          cv.width = Math.round(wd); cv.height = Math.round(side);
          const x = cv.getContext('2d'); x.imageSmoothingEnabled = false;
          x.drawImage(buf, Math.max(0, Math.min(s.w - wd, s.cx - wd / 2)), Math.max(0, Math.min(s.h - side, s.cy - side / 2)), wd, side, 0, 0, cv.width, cv.height);
        }).catch(() => {}).finally(() => { busy = false; });
      };
      clearInterval(live); live = setInterval(frame, 300); frame();   // the app gives it one every 200 ms at most
      return box;
    }
    // the round button's icon (the skin's, once it's there: tick swaps a placeholder for it)
    const icon = k => { const b = document.querySelector(`.w [data-w="${k}"] svg`); if (b) return b.cloneNode(true); const t = document.createElement('template'); t.innerHTML = SVG.more; t.content.firstChild.classList.add('ph'); return t.content.firstChild; };
    function deckView() {
      const box = h('div', 'deck');
      for (const t of deck([...$('shop').querySelectorAll('[data-buy]')].map(b => ({ key: b.dataset.buy })), true)) {
        let b;
        if (t.kind === 'kaio') { b = button('tile big t-kaio', icon('kaio'), h('span')); b.addEventListener('click', () => $('kaio').click()); }
        else if (t.kind === 'itx') { b = button('tile big t-itx', icon('itx'), h('span')); b.addEventListener('click', () => $('itxBtn').click()); }
        else if (t.kind === 'more') { b = button('tile more', count('tree'), h('span', '', 'ki tree', h('br'), 'heroes', h('br'), 'achievements')); b.addEventListener('click', () => { sheet = true; build(); }); }
        else { b = button('tile', h('div', '', h('b'), h('br'), h('i')), h('em')); b.dataset.key = t.key; b.addEventListener('click', () => { const r = $('shop').querySelector(`[data-buy="${t.key}"]`); if (r && !r.disabled) r.click(); }); }
        b.dataset.tile = t.kind; box.append(b);
      }
      return box;
    }
    function bulkTile() {
      const b = button('card bulkt', h('span', 'lbl', 'bulk'), h('b')); b.dataset.bulk = '1';
      b.addEventListener('click', () => { const bs = [...$('shop').querySelectorAll('.bulk [data-n]')], i = bs.findIndex(x => x.getAttribute('aria-pressed') === 'true'); if (bs.length) bs[(i + 1) % bs.length].click(); tick(); });
      return b;
    }
    // each part into its column. The chip and round buttons stay in .w (its clicks are widget.js's), which lays out as if
    // they were the column's own (display: contents)
    const PART = {
      hud: c => { if (w()) c.append(w()); },
      top: c => { const r = h('div', 'row top'); if (w()) r.append(w()); if (layout === 4) r.append(bulkTile()); c.append(r); },
      shop: (c, col, plan) => { const box = h('div', col.parts.length === 1 ? 'card pane' : 'card'); if (col.parts.length > 1) box.style.flex = 'none'; box.append(shop(plan.shop || [])); c.append(heading('upgrades', 'up'), box); },
      tree: (c, col, plan) => { const box = h('div', 'card pane'); box.append(menuPart('tree', { tight: plan.tight })); c.append(heading('ki tree', 'tree'), box); },
      heroes: c => { const box = h('div', 'card'); box.style.flex = 'none'; box.append(menuPart('heroes', { row: true })); c.append(heading('heroes', 'heroes'), box); },
      ach: c => { const box = h('div', 'card pane'); box.append(menuPart('ach')); c.append(heading('achievements', 'ach'), box); },
      live: c => c.append(liveView()),
      deck: c => c.append(deckView()),
      toast: c => c.append(h('div', 'toastslot')),   // the panel's toasts show here (#toast is fixed to the foot)
    };
    if (PARTS.some(p => !PART[p])) throw new Error('control.js: a part with no builder');

    function build() {
      const s = api.settings() || {};
      layout = pick(s.control_layout); root.dataset.cc = layout; cc.dataset.l = layout;
      const plan = PLAN[layout], keep = [w(), $('shop')];
      clearInterval(live); live = null;
      cc.textContent = '';
      for (const k of keep) if (k && !k.isConnected) document.body.append(k);   // parked until placed
      for (const col of plan.cols) {
        const c = h('div', 'col');
        if (col.w) c.style.flex = `0 0 ${col.w}px`;
        for (const p of col.parts) if (p.startsWith('tabs:')) c.append(tabs(p.slice(5).split(','), plan)); else PART[p](c, col, plan);
        cc.append(c);
      }
      if (sheet && plan.more) {
        const sh = h('div', 'card sheet'), t = tabs(plan.more, plan), x = button('x', '×');
        x.setAttribute('aria-label', 'close'); x.addEventListener('click', () => { sheet = false; build(); });
        t.querySelector('.tabs').append(x); t.classList.remove('card'); sh.append(t); cc.append(sh);
      }
      $('shop').style.display = $('shop').closest('#cc') ? '' : 'none';   // not placed: read by the deck, not shown
      deckKeys = shopKeys(); menuAt = -1; tick();
    }

    // ── kept up to date: the badges, the menus when hud.js has new ones, the deck's tiles, the sound button
    // a name too long for its tile gets smaller, whole words kept, rather than clipped or broken
    const fit = e => { e.style.fontSize = ''; for (let px = parseFloat(getComputedStyle(e).fontSize); e.scrollWidth > e.clientWidth && px > 7; ) e.style.fontSize = (px -= .5) + 'px'; };
    addEventListener('resize', () => cc.querySelectorAll('.tile b, .tile.big span').forEach(fit));
    const shopKeys = () => [...$('shop').querySelectorAll('[data-buy]')].map(b => b.dataset.buy).join();
    let deckKeys = '';
    function tick() {
      if (document.hidden || !layout) return;
      if (layout === 4 && shopKeys() !== deckKeys) return build();   // the deck has a tile for every upgrade there is
      const chip = cc.querySelector('.dw-chip'); if (chip) chip.classList.toggle('sm', layout !== 1);
      const m = game.menu(), n = counts(m, $('shopCount') ? +$('shopCount').textContent || 0 : 0);
      for (const b of cc.querySelectorAll('b[data-n]')) {
        const k = b.dataset.n, v = n[k];
        const t = !b.closest('.lbl') || !v ? v : k === 'up' ? v + ' to buy' : k === 'tree' ? v + ' to learn' : v;
        if (b.textContent !== String(t)) b.textContent = t;
      }
      if (api.fresh() !== menuAt) { menuAt = api.fresh(); for (const b of cc.querySelectorAll('[data-menu]')) MENUS[b.dataset.menu](b); }
      // the deck: the upgrades as the list shows them, and the boosts as their round buttons
      for (const t of cc.querySelectorAll('.tile[data-key]')) {
        const r = $('shop').querySelector(`[data-buy="${t.dataset.key}"]`); if (!r) continue;
        const name = r.querySelector('b').firstChild.textContent.trim(), lv = (r.querySelector('[data-lv]') || {}).textContent || '', cost = (r.querySelector('[data-cost]') || {}).textContent || '';
        const q = s => t.querySelector(s);
        if (q('b').textContent !== name) { q('b').textContent = name; fit(q('b')); }
        if (q('i').textContent !== lv) q('i').textContent = lv;
        if (q('em').textContent !== cost) q('em').textContent = cost;
        t.classList.toggle('can', !r.disabled); t.classList.toggle('no', r.disabled); t.hidden = r.hidden;
      }
      for (const [k, id] of [['kaio', 'kaio'], ['itx', 'itxBtn']]) {
        const t = cc.querySelector(`.t-${k}`), src = $(id), round = document.querySelector(`.w [data-w="${k}"]`); if (!t || !src) continue;
        t.hidden = src.hidden; t.disabled = src.disabled;
        const ph = t.querySelector('svg.ph'); if (ph && round) ph.replaceWith(round.querySelector('svg').cloneNode(true));   // the skin's icons came in after the deck
        const name = CONTROL.label(src.title || src.getAttribute('aria-label'), src.firstChild && src.firstChild.textContent);
        const sp = t.querySelector('span'), left = src.querySelector('small');
        const text = name + (left ? ' ' + left.textContent : ''); if (sp.textContent !== text) { sp.textContent = text; fit(sp); }
        if (round) for (const v of ['--f', '--kc', '--kp']) { const x = round.style.getPropertyValue(v) || src.style.getPropertyValue(v); if (x) t.style.setProperty(v, x); else t.style.removeProperty(v); }
      }
      const bt = cc.querySelector('[data-bulk] b'), on = $('shop').querySelector('.bulk [aria-pressed="true"]');
      if (bt && on) bt.textContent = on.textContent;
      const s = api.settings(), snd = cc.querySelector('.w [data-w="snd"]');
      if (s && snd) snd.setAttribute('aria-pressed', String(s.sound === true));
    }

    // the game's sound plays in the game's window: here the button only says so to the app
    cc.addEventListener('click', e => {
      const b = e.target.closest('[data-w="snd"]'); if (!b) return;
      e.preventDefault(); e.stopImmediatePropagation();
      const s = api.settings(); api.invoke('set_sound', { on: !(s && s.sound === true) }).catch(() => {});
    }, true);
    for (const t of ['pointermove', 'pointerdown', 'keydown']) addEventListener(t, () => { usedAt = performance.now(); }, { passive: true, capture: true });

    document.body.append(cc);
    // widget.js builds .w once the page's script has run: then it's laid out. Kept up four times a second while it's
    // used, once a second otherwise
    setTimeout(build);
    (function again() { tick(); setTimeout(again, active() ? 250 : 1000); })();
    document.addEventListener('visibilitychange', tick);
    // a new layout picked: laid out again
    return { layout: n => { if (pick(n) !== layout) build(); } };
  }

  return { LAYOUTS, pick, role, shows, lens, toastsOnPanel, PLAN, PARTS, TABS, parts, active, deck, label, counts, CSS, mount };
})();
if (typeof module === 'object' && module.exports) module.exports = CONTROL;
