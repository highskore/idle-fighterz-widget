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
    if (who === 'panel') return Object.assign({}, s, { skin: 'drawer', layout: 7, hud: 'always', mode: 'window', opacity: 1, clouds: 'never' });
    return Object.assign({}, s, layout === 5 ? { skin: 'classic', layout: 7 } : { skin: 'drawer' }, { hud: 'always' });
  }

  // what each layout's panel holds: its columns of parts, and the tabs ('tabs') or sheet ('more') for the rest
  const PLAN = {
    1: [['chip', 'rail', 'tabs:up,tree,heroes,ach']],
    2: [['top', 'shop', 'tree', 'heroes', 'toast']],
    3: [['live', 'chip', 'rail'], ['tabs:up,tree,heroes,ach']],
    4: [['top', 'deck']],
    5: [['chip', 'rail', 'heroes', 'ach'], ['shop'], ['tree']],
  };
  const parts = layout => PLAN[pick(layout)].flat().flatMap(p => (p.startsWith('tabs:') ? p.slice(5).split(',') : [p]));

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
html.control:not([data-cc="3"]) #cc .w [data-w="share"] { display: none !important; }
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
#cc .tile b { font-weight: 400; font-size: 12px; line-height: 1.2; }
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
  // achievements drawn here from the game's menus. api: { game (hud.js's), invoke, settings() (as the app sent them) }
  function mount(api) {
    const { game } = api, root = document.documentElement, $ = id => document.getElementById(id);
    const el = (tag, cls, html = '') => { const e = document.createElement(tag); if (cls) e.className = cls; e.innerHTML = html; return e; };
    const cc = el('div'); cc.id = 'cc';
    let layout = 0, tab = 'up', sheet = false, menuKey = '', live = null;
    const thumbs = {};
    const thumb = id => thumbs[id] || (thumbs[id] = game.thumb(id));

    // ── the menus, as the page's menu draws them
    function tree(body, tight) {
      const m = game.menu();
      body.className = 'menu-body tree' + (tight ? ' tight' : ''); body.textContent = '';
      const head = el('p', 'tree-head', `<span><b>${num(m.seeds)}</b> ki seeds${m.seeds || m.spent ? ` · ×${m.boost} on everything` : ' — ascend to earn some'}</span>`);
      const right = el('span');
      if (m.spent) { const b = el('button', '', 'reset'); b.type = 'button'; b.addEventListener('click', () => { game.respec(); later(); }); right.append(b); }
      if (game.angels().gods) { const b = el('button', '', 'angels'); b.type = 'button'; b.addEventListener('click', () => { angelsOn = true; draw(); }); right.append(b); }
      head.append(right); body.append(head);
      (m.branches || []).forEach((br, col) => m.tree.filter(n => n.branch === br.name).forEach((n, row) => {
        const b = el('button', 'node' + (n.can ? ' can' : '') + (n.lvl ? ' on' : '') + (n.open ? '' : ' locked') + (n.tier === 3 ? ' cap' : ''));
        b.type = 'button'; b.disabled = !n.can;
        b.innerHTML = `<i>${row ? '' : br.name + ' · ' + br.pts + ' pts'}</i><b>${n.name}</b><span>${n.what}</span><em>${!n.open ? `needs ${n.need} in ${n.branch}` : (n.max === Infinity || n.max === null ? 'lv ' + n.lvl : n.lvl + '/' + n.max) + (n.lvl < n.max ? ' · ' + num(n.cost) + ' seed' + (n.cost > 1 ? 's' : '') : '')}</em>`;
        b.style.gridColumn = col + 1; b.style.gridRow = row + 2;
        b.addEventListener('click', () => { game.learn(n.id); later(); });
        body.append(b);
      }));
    }
    let angelsOn = false;
    function angels(body) {
      const an = game.angels();
      body.className = 'menu-body tree angels'; body.textContent = '';
      const head = el('p', 'tree-head', `<span><b>${num(an.godki)}</b> god ki · ×${an.boost} on everything</span>`), right = el('span');
      if (an.auto) { const b = el('button', '', 'self-ascend: ' + an.auto); b.type = 'button'; b.addEventListener('click', () => { game.autoNext(); later(); }); right.append(b); }
      const back = el('button', '', 'ki tree'); back.type = 'button'; back.addEventListener('click', () => { angelsOn = false; draw(); }); right.append(back);
      head.append(right); body.append(head);
      ['whis', 'vados'].forEach((who, col) => an.list.filter(a => a.who === who).forEach((a, row) => {
        const b = el('button', 'node' + (a.can ? ' can' : '') + (a.on ? ' on' : '') + (a.open ? '' : ' locked') + (a.tier === 2 ? ' cap' : ''));
        b.type = 'button'; b.disabled = !a.can;
        b.innerHTML = `<i>${row ? '' : who}</i><b>${a.name}</b><span>${a.what}</span><em>${a.on ? 'learned' : !a.open ? `needs ${a.need} from ${who}` : a.cost + ' god ki'}</em>`;
        b.style.gridColumn = col + 1; b.style.gridRow = row + 2;
        b.addEventListener('click', () => { game.learnAngel(a.id); later(); });
        body.append(b);
      }));
    }
    function heroes(body, row) {
      body.className = row ? 'carousel' : 'menu-body chars'; body.textContent = '';
      for (const c of game.menu().chars) {
        const b = el('button', 'char' + (c.on ? ' on' : '') + (c.open ? '' : ' locked')); b.type = 'button';
        b.append(thumb(c.id)); b.insertAdjacentHTML('beforeend', `<b>${c.name}</b><span>${c.on ? 'playing' : c.open ? 'tap to play' : 'unlock: ' + c.need}</span>`);
        if (c.open) b.addEventListener('click', () => { game.pick(c.id); later(); });
        body.append(b);
      }
    }
    function ach(body) {
      body.className = 'menu-body'; body.dataset.icons = 'pills'; body.textContent = '';
      for (const a of game.menu().ach) {
        const d = el('button', 'ach' + (a.done ? ' done' : ''), `<b>${a.name}</b><span>${a.text}</span>`); d.type = 'button'; d.title = a.text;
        d.addEventListener('click', () => d.classList.toggle('open'));
        body.append(d);
      }
    }
    const MENUS = { tree: b => (angelsOn ? angels(b) : tree(b, b.classList.contains('tight') || b.dataset.tight === '1')), heroes: b => heroes(b, b.dataset.row === '1'), ach };
    // what a click changed comes back with the menus hud.js fetches again: draw once it's in
    const later = () => setTimeout(() => { menuKey = ''; tick(); }, 120);

    // ── the parts
    const w = () => document.querySelector('.w');
    const shop = (...cls) => { const s = $('shop'); s.classList.remove('desc', 'grid2', 'nobulk'); s.classList.add(...cls.filter(Boolean)); return s; };
    const menuPart = (kind, opts = {}) => { const b = el('div', 'menu-body'); b.dataset.menu = kind; if (opts.tight) b.dataset.tight = '1'; if (opts.row) b.dataset.row = '1'; return b; };
    const heading = (text, key) => el('div', 'lbl', `<span>${text}</span><b data-n="${key}"></b>`);
    const NAMES = { up: 'upgrades', tree: 'ki tree', heroes: 'heroes', ach: 'achievements' };
    function tabs(list, shopCls) {
      if (!list.includes(tab)) tab = list[0];
      const box = el('div', 'card grow'), head = el('div', 'tabs'), pane = el('div', 'pane');
      for (const k of list) { const b = el('button', k === tab ? 'on' : '', `${NAMES[k]} <b data-n="${k}"></b>`); b.type = 'button'; b.dataset.tab = k; head.append(b); }
      head.addEventListener('click', e => { const b = e.target.closest('[data-tab]'); if (b) { tab = b.dataset.tab; build(); } });
      pane.append(tab === 'up' ? shop(...shopCls) : menuPart(tab, { tight: false }));
      box.append(head, pane);
      return box;
    }
    function liveView() {
      const box = el('div', 'live', '<canvas></canvas><em>live</em>'), cv = box.firstChild;
      const buf = document.createElement('canvas');
      let busy = false;
      const frame = () => {
        if (busy || document.hidden || !box.isConnected) return; busy = true;
        Promise.all([api.invoke('planet_spot').then(JSON.parse), api.invoke('planet_pixels')]).then(([s, px]) => {
          if (!px || px.byteLength !== s.w * s.h * 4) return;   // between frames the app sends nothing (control.rs)
          buf.width = s.w; buf.height = s.h; buf.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(px), s.w, s.h), 0, 0);
          // the planet and some sky round it, at the box's shape
          const k = cv.clientWidth / Math.max(1, cv.clientHeight), side = Math.min(s.h, Math.max(s.r * 3.2, 40)), wd = Math.min(s.w, side * k);
          cv.width = Math.round(wd); cv.height = Math.round(side);
          const x = cv.getContext('2d'); x.imageSmoothingEnabled = false;
          x.drawImage(buf, Math.max(0, Math.min(s.w - wd, s.cx - wd / 2)), Math.max(0, Math.min(s.h - side, s.cy - side / 2)), wd, side, 0, 0, cv.width, cv.height);
        }).catch(() => {}).finally(() => { busy = false; });
      };
      clearInterval(live); live = setInterval(frame, 260); frame();
      return box;
    }
    const icon = k => { const b = document.querySelector(`.w [data-w="${k}"] svg`); return b ? b.outerHTML : SVG.more.replace('<svg', '<svg class="ph"'); };
    function deckView() {
      const box = el('div', 'deck');
      const rows = [...$('shop').querySelectorAll('[data-buy]')].map(b => ({ key: b.dataset.buy }));
      for (const t of deck(rows, true)) {
        const b = el('button', 'tile'); b.type = 'button'; b.dataset.tile = t.kind; if (t.key) b.dataset.key = t.key;
        if (t.kind === 'kaio') { b.className = 'tile big t-kaio'; b.innerHTML = `${icon('kaio')}<span></span>`; b.addEventListener('click', () => $('kaio').click()); }
        else if (t.kind === 'itx') { b.className = 'tile big t-itx'; b.innerHTML = `${icon('itx')}<span></span>`; b.addEventListener('click', () => $('itxBtn').click()); }
        else if (t.kind === 'more') { b.className = 'tile more'; b.innerHTML = '<b data-n="tree"></b><span>ki tree<br>heroes<br>achievements</span>'; b.addEventListener('click', () => { sheet = true; if (tab === 'up') tab = 'tree'; build(); }); }
        else { b.innerHTML = '<div><b></b><br><i></i></div><em></em>'; b.addEventListener('click', () => { const r = $('shop').querySelector(`[data-buy="${t.key}"]`); if (r && !r.disabled) r.click(); }); }
        box.append(b);
      }
      return box;
    }
    function bulkTile() {
      const b = el('button', 'card bulkt', '<span class="lbl">bulk</span><b></b>'); b.type = 'button'; b.dataset.bulk = '1';
      b.addEventListener('click', () => { const bs = [...$('shop').querySelectorAll('.bulk [data-n]')], i = bs.findIndex(x => x.getAttribute('aria-pressed') === 'true'); if (bs.length) bs[(i + 1) % bs.length].click(); tick(); });
      return b;
    }
    // the chip and the round buttons stay in .w (its clicks are widget.js's); .w lays out as if they were the column's own
    function chipRail(col, wrap) {
      const x = w(); if (!x) return;
      col.append(x);
      const chip = x.querySelector('.dw-chip'), rail = x.querySelector('.dw-rail');
      if (wrap) { if (chip) chip.style.order = 0; if (rail) rail.style.order = 1; }
    }

    function build() {
      const s = api.settings() || {};
      layout = pick(s.control_layout); root.dataset.cc = layout; cc.dataset.l = layout;
      const keep = [w(), $('shop')];
      clearInterval(live); live = null;
      cc.textContent = '';
      for (const k of keep) if (k && !k.isConnected) document.body.append(k);   // parked until placed
      const col = (...kids) => { const c = el('div', 'col'); c.append(...kids.filter(Boolean)); return c; };
      if (layout === 1) {
        const c = col(); chipRail(c); c.append(tabs(['up', 'tree', 'heroes', 'ach'], ['desc'])); cc.append(c);
      } else if (layout === 2) {
        const top = el('div', 'row top'), c = col(top);
        top.append(w());
        const sh = el('div', 'card'); sh.style.flex = 'none'; sh.append(shop('grid2', 'nobulk'));
        const tr = el('div', 'card pane'), th = el('div', 'card'); th.style.flex = 'none';
        tr.append(menuPart('tree', { tight: true })); th.append(menuPart('heroes', { row: true }));
        c.append(heading('upgrades', 'up'), sh, heading('ki tree', 'tree'), tr, heading('heroes', 'heroes'), th, el('div', 'toastslot'));   // its toasts show here
        cc.append(c);
      } else if (layout === 3) {
        const left = col(liveView()); left.style.flex = '0 0 280px'; chipRail(left);
        cc.append(left, col(tabs(['up', 'tree', 'heroes', 'ach'], ['grid2'])));
      } else if (layout === 4) {
        const top = el('div', 'row top'); top.append(w(), bulkTile());
        cc.append(col(top, deckView()));
        if (sheet) {
          const sh = el('div', 'card sheet'), t = tabs(['tree', 'heroes', 'ach'], []), x = el('button', 'x', '×');
          x.type = 'button'; x.setAttribute('aria-label', 'close'); x.addEventListener('click', () => { sheet = false; build(); });
          t.querySelector('.tabs').append(x); t.classList.remove('card'); sh.append(t); cc.append(sh);
        }
        $('shop').classList.remove('desc', 'grid2'); document.body.append($('shop'));   // read by the tiles, not shown
        $('shop').style.display = 'none';
      } else {
        const c1 = col(); c1.style.flex = '0 0 250px'; chipRail(c1);
        const he = el('div', 'card'); he.style.flex = 'none'; he.append(menuPart('heroes', { row: true }));
        const ac = el('div', 'card pane'); ac.append(menuPart('ach'));
        c1.append(heading('heroes', 'heroes'), he, heading('achievements', 'ach'), ac);
        const up = el('div', 'card pane'); up.append(shop());
        const c2 = col(heading('upgrades', 'up'), up); c2.style.flex = '0 0 260px';
        const tr = el('div', 'card pane'); tr.append(menuPart('tree', { tight: true }));
        cc.append(c1, c2, col(heading('ki tree', 'tree'), tr));
      }
      if (layout !== 4) $('shop').style.display = '';
      deckKeys = shopKeys(); menuKey = ''; tick();
    }

    // ── kept up to date: the badges, the menus when they change, the deck's tiles, the sound button
    const shopKeys = () => [...$('shop').querySelectorAll('[data-buy]')].map(b => b.dataset.buy).join();
    let deckKeys = '';
    function tick() {
      if (document.hidden || !layout) return;
      if (layout === 4 && shopKeys() !== deckKeys) return build();   // the deck has a tile for every upgrade there is
      const chip = cc.querySelector('.dw-chip'); if (chip) chip.classList.toggle('sm', layout !== 1);
      const m = game.menu(), an = game.angels(), n = counts(m, $('shopCount') ? +$('shopCount').textContent || 0 : 0);
      for (const b of cc.querySelectorAll('b[data-n]')) {
        const k = b.dataset.n, v = n[k];
        const t = !b.closest('.lbl') || !v ? v : k === 'up' ? v + ' to buy' : k === 'tree' ? v + ' to learn' : v;
        if (b.textContent !== String(t)) b.textContent = t;
      }
      const key = JSON.stringify([m, an, angelsOn]);
      if (key !== menuKey) { menuKey = key; for (const b of cc.querySelectorAll('[data-menu]')) MENUS[b.dataset.menu](b); }
      // the deck: the upgrades as the list shows them, and the boosts as their round buttons
      for (const t of cc.querySelectorAll('.tile[data-key]')) {
        const r = $('shop').querySelector(`[data-buy="${t.dataset.key}"]`); if (!r) continue;
        const name = r.querySelector('b').firstChild.textContent.trim(), lv = (r.querySelector('[data-lv]') || {}).textContent || '', cost = (r.querySelector('[data-cost]') || {}).textContent || '';
        const q = s => t.querySelector(s);
        if (q('b').textContent !== name) q('b').textContent = name;
        if (q('i').textContent !== lv) q('i').textContent = lv;
        if (q('em').textContent !== cost) q('em').textContent = cost;
        t.classList.toggle('can', !r.disabled); t.classList.toggle('no', r.disabled); t.hidden = r.hidden;
      }
      for (const [k, id] of [['kaio', 'kaio'], ['itx', 'itxBtn']]) {
        const t = cc.querySelector(`.t-${k}`), src = $(id), round = document.querySelector(`.w [data-w="${k}"]`); if (!t || !src) continue;
        t.hidden = src.hidden; t.disabled = src.disabled;
        const ph = t.querySelector('svg.ph'); if (ph && round) ph.outerHTML = round.querySelector('svg').outerHTML;   // the skin's icons came in after the deck
        const name = CONTROL.label(src.title || src.getAttribute('aria-label'), src.firstChild && src.firstChild.textContent);
        const sp = t.querySelector('span'), left = src.querySelector('small');
        const text = name + (left ? ' ' + left.textContent : ''); if (sp.textContent !== text) sp.textContent = text;
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

    document.body.append(cc);
    // widget.js builds .w once the page's script has run: then it's laid out
    setTimeout(build);
    setInterval(tick, 250);
    document.addEventListener('visibilitychange', tick);
    // a new layout picked: laid out again
    return { layout: n => { if (pick(n) !== layout) build(); }, scene: () => (layout === 3 ? cc.querySelector('.live canvas') : null) };
  }

  return { LAYOUTS, pick, role, shows, lens, toastsOnPanel, PLAN, parts, deck, label, counts, CSS, mount };
})();
if (typeof module === 'object' && module.exports) module.exports = CONTROL;
