// The planet on the Rust core (cargo feature rust-core): planet-core runs in the app (src-tauri/src/planet.rs), which
// steps it, draws it with planet-render and sends a `planet` event with each frame's message. This hands page.html a
// `game` object with game.js's API over the app's planet_* commands and drives page.html's `ui` from those messages.
// It's game/rust/planet-web/shim.js with `invoke` in place of the wasm `Planet`, and without its lockstep twin. The
// frames go either to a layer the app keeps under this page (macOS, feature native-renderer) or onto this canvas.
// The control centre (control.js) loads this page twice: in the game's window and in its own (html.control, the panel)

// which page this is, and the settings as it takes them (control.js lens): read before widget.js asks for them
const CC = (() => {
  const T = window.__TAURI__, control = document.documentElement.classList.contains('control');
  let last = null;
  const who = () => CONTROL.role(control, last);
  const see = s => { last = s; return CONTROL.lens(s, who()); };
  if (T) {
    const invoke = T.core.invoke, listen = T.event.listen;
    // the menu-bar number, the page's state and notifications are the game window's to send
    const QUIET = ['tray_title', 'page_state', 'notify'];
    T.core = Object.assign({}, T.core, { invoke: (cmd, args, o) => (control && QUIET.includes(cmd) ? Promise.resolve() : cmd === 'get_settings' ? invoke(cmd, args, o).then(see) : invoke(cmd, args, o)) });
    T.event = Object.assign({}, T.event, { listen: (ev, fn, o) => listen(ev, ev === 'settings' ? e => fn(Object.assign({}, e, { payload: see(e.payload) })) : fn, o) });
  }
  return { control, who, layout: () => CONTROL.pick(last && last.control_layout), settings: () => last, invoke: (...a) => T.core.invoke(...a) };
})();

GAMES.push({
  id: 'planet',
  mount(st, ui) {
    const panel = CC.control;
    const tauri = window.__TAURI__, call = (cmd, args) => {
      // logged, so a refused call shows (most callers don't wait on it); a caller that does still sees it fail
      const p = tauri.core.invoke('planet_' + cmd, args);
      p.catch(e => console.error('planet_' + cmd, e));
      return p;
    };
    const ptr = e => { const [bx, by] = st.toBuf(e); return { x: e.clientX, y: e.clientY, bx, by, id: e.pointerId }; };

    // page.html reads these synchronously, so they're kept from the last time anything changed
    const view = {
      menu: { ach: [], tree: [], branches: [], chars: [], seeds: 0, spent: 0, boost: 1 }, angels: { gods: false, godki: 0, boost: 1, auto: '', list: [] },
      ascend_info: { ok: false, seeds: 0 }, god_info: {}, zeno_info: {}, card: {},
    };
    let fresh = 0;
    const refresh = () => { fresh = performance.now(); return Promise.all(Object.keys(view).map(k => call(k).then(s => { view[k] = JSON.parse(s); })))
      .then(() => { for (const n of view.menu.tree) if (n.max === null) n.max = Infinity; }); };
    // after a menu button: the menu again, once what it changed is in
    const rerender = () => refresh().then(() => { const tab = document.querySelector('[data-mt].on'); if (tab && !document.getElementById('menu').hidden) tab.click(); });

    // the frames: on the app's layer under this page (`native`), or fetched and put on this canvas when a message says so.
    // st.buf keeps what's drawn either way (or just where, on the layer), for widget.js's empty-sky drag test
    let native = false, fetching = false, again = false;
    const pixels = () => {
      if (panel) return;   // the panel draws no planet (3's live one is control.js's)
      if (fetching) { again = true; return; }
      fetching = true;
      call('pixels').then(b => {
        if (native || b.byteLength !== st.W * st.H * 4) return;   // a resize or the layer got there first
        st.ctx.putImageData(new ImageData(new Uint8ClampedArray(b), st.W, st.H), 0, 0);
        st.buf = new Uint32Array(b);
      }).catch(e => console.error(e)).finally(() => { fetching = false; if (again) { again = false; pixels(); } });
    };
    let maskAt = -1e9;
    const mask = () => {
      const now = performance.now(); if (!native || now - maskAt < 100) return; maskAt = now;
      call('mask').then(b => {
        const m = new Uint8Array(b), n = st.W * st.H; if (m.length !== Math.ceil(n / 8)) return;
        const buf = new Uint32Array(n); for (let k = 0; k < n; k++) if (m[k >> 3] >> (k & 7) & 1) buf[k] = 0x01000000;
        st.buf = buf;
      }).catch(e => console.error(e));
    };
    if (!panel) {
      st.el.addEventListener('pointerenter', mask); st.el.addEventListener('pointermove', mask);
      tauri.event.listen('pointer', ({ payload }) => { if (payload.inside && payload.x >= 0) mask(); });   // another app active: no pointer moves (x -1: over the control centre)
    }

    // how the page shows it: floating or window mode, the clouds widget.js picks (they come and go with the pointer),
    // reduced motion, where the canvas sits and the page's opacity. Several changes at once go as one call
    let viewDue = false;
    const show = () => {
      if (viewDue || panel) return; viewDue = true;
      queueMicrotask(() => {
        viewDue = false;
        const r = st.cv.getBoundingClientRect();
        call('view', { bare: BARE, clouds: st.clouds || '', amount: st.cloudAmt || 2, calm: !!st.calm, x: r.left, y: r.top, w: r.width, h: r.height, opacity: +(document.body.style.opacity || 1) })
          .then(n => { if (n === native) return; native = n; maskAt = -1e9; if (native) { st.ctx.clearRect(0, 0, st.W, st.H); mask(); } else pixels(); })
          .catch(e => console.error(e));
      });
    };
    for (const k of ['clouds', 'cloudAmt']) {
      let v = st[k];
      Object.defineProperty(st, k, { get: () => v, set: x => { if (x !== v) { v = x; show(); } }, configurable: true });
    }
    new MutationObserver(show).observe(document.body, { attributes: true, attributeFilter: ['style'] });

    // planet-core's messages, onto page.html's ui, in order. The menus and boxes page.html reads synchronously are
    // fetched again with the shop, a cue or a new world, and at most once a second for the HUD alone
    let night = false, shen = false, look = '', itxOwn = false, queue = refresh();
    const back = { wish: key => call('pick_wish', { key }), itxPick: w => call('itx_jump', { w }) };
    const deliver = m => {
      if ('world' in m) { ui.hell && ui.hell(false); ui.world && ui.world(m.world); }
      if (m.hud) {
        ui.update(m.hud); itxOwn = !!(m.hud.itx && m.hud.itx.own);
        if (m.hud.night !== night) { night = m.hud.night; ui.night && ui.night(night); }
        if (m.hud.shen !== shen) document.body.classList.toggle('shen', shen = m.hud.shen);
        if (m.hud.look !== look) { document.body.classList.remove('jelly', 'earth'); look = m.hud.look; if (look) document.body.classList.add(look); }
      }
      if (m.shop) ui.shop(m.shop);
      // with the control centre, each toast and cue shows on one page: the game's window or the panel (control.js)
      for (const [text, ms] of m.events) if (shows('toast')) ui.event(text, ms);
      for (const [cue, ...args] of m.cues) if (!shows(cue)) continue; else if (back[cue]) ui[cue] && ui[cue](args[0], back[cue]); else if (typeof ui[cue] === 'function') ui[cue](...args);
    };
    const shows = what => CONTROL.shows(CC.who(), CC.layout(), what);
    tauri.event.listen('planet', ({ payload }) => {
      const m = JSON.parse(payload);
      if (m.px) { if (native) show(); else pixels(); }   // while the layer has them: stale, or the layer failed; ask the app
      // the panel fetches them at most once a second while it's used, every 5 s otherwise (a cue or a new world at once)
      const since = performance.now() - fresh, every = panel ? (CONTROL.active() ? 1000 : 5000) : 1000;
      const stale = m.cues.length || 'world' in m || (panel ? (m.shop || m.hud) && since >= every : m.shop || (m.hud && since >= every));
      queue = queue.then(() => (stale ? refresh() : null)).then(() => deliver(m)).catch(e => console.error(e));
    });
    // n: how many (shift-click 10, alt-click 0 for max); a plain click is one buy
    ui.onBuy((key, n = 1) => (n === 1 ? call('buy', { key }) : call('buy_n', { key, n })));
    // the app's own news (an update is out, a photo saved, the away recap), as the game's toasts
    tauri.event.listen('notice', ({ payload }) => { if (shows('toast')) ui.event(payload, 8000); });

    // photo mode (the tray, or P): the app pauses the game and the HUD steps aside for the frame; any key or click
    // carries on, and goes no further
    const root = document.documentElement, photoCss = document.createElement('style');
    photoCss.textContent = 'html.photo #game > :not(.stage), html.photo .menu { visibility: hidden !important; transition: none !important; }';
    document.head.append(photoCss);
    tauri.event.listen('photo', ({ payload }) => root.classList.toggle('photo', !!payload));
    let swallow = false;
    const leave = e => {
      if (e.type === 'click' && swallow) { swallow = false; e.preventDefault(); e.stopImmediatePropagation(); return; }
      if (!root.classList.contains('photo') || e.type === 'click') return;
      e.preventDefault(); e.stopImmediatePropagation();
      if (e.type === 'pointerdown') swallow = true;
      root.classList.remove('photo'); tauri.core.invoke('photo_exit');
    };
    for (const t of ['pointerdown', 'keydown', 'click']) addEventListener(t, leave, true);

    // the game's sound lives in the page; the app keeps it with the settings so a profile can carry it
    const snd = document.getElementById('snd'), soundOn = () => snd.getAttribute('aria-pressed') === 'true';
    const tellSound = () => tauri.core.invoke('set_sound', { on: soundOn() }).catch(() => {});
    const applySound = st => {
      if (typeof st.sound !== 'boolean' || st.sound === soundOn()) return;
      snd.click(); document.querySelector('[data-w="snd"]')?.setAttribute('aria-pressed', String(soundOn()));
    };
    // (the panel plays none: its sound button tells the app, control.js)
    if (!panel) {
      for (const b of [snd, document.getElementById('sndM')]) b?.addEventListener('click', () => setTimeout(tellSound));
      tauri.core.invoke('get_settings').then(st => (typeof st.sound === 'boolean' ? applySound(st) : tellSound()));
      tauri.event.listen('settings', ({ payload }) => applySound(payload));
    }

    if (!panel) {
      st.cv.addEventListener('pointerdown', e => call('pointer_down', { ...ptr(e), button: e.button, touch: e.pointerType === 'touch' }));
      st.cv.addEventListener('pointermove', e => call('pointer_move', ptr(e)));
      st.cv.addEventListener('pointerup', e => { const { bx, by, id } = ptr(e); call('pointer_up', { bx, by, id }); });
      st.cv.addEventListener('pointercancel', () => call('pointer_cancel'));
    }
    st.cv.addEventListener('contextmenu', e => { if (itxOwn) e.preventDefault(); });   // right-click is instant transmission's once it's owned
    // fullscreen mode (settings): Esc leaves it for the mode it came from once no panel is open (the page's own Esc closes
    // the upgrades, the menu, a box or the drawer first), and F11 toggles it (⌃⌘F on macOS, where F11 shows the desktop).
    // The window fills its display then: square corners, and no dragging it. ⌘/ctrl +, − and 0 step the UI size and ⌥/alt
    // the planet size (hud-keys.js), kept in the settings. A toast keeps to the bottom (a buff line mustn't give it a top too)
    const fullCss = document.createElement('style');
    fullCss.textContent = 'html.full.widget.windowed #game { border: 0; border-radius: 0; } html.full.widget #toast { top: auto; }';
    document.head.append(fullCss);
    let full = false, mac = false;
    // the control centre: the game's window shows what the layout leaves it (html.cc), the panel lays out the HUD
    const ccCss = document.createElement('style'); ccCss.textContent = CONTROL.CSS; document.head.append(ccCss);
    let ccUi = null;
    const pickMode = s => {
      full = s.mode_pick === 'fullscreen'; root.classList.toggle('full', full);
      const game = CC.who() === 'game'; root.classList.toggle('cc', game);
      if (game || panel) root.dataset.cc = CC.layout(); else delete root.dataset.cc;
      if (ccUi) ccUi.layout(CC.layout());
    };
    tauri.core.invoke('get_settings').then(s => { mac = s.os === 'macos'; pickMode(s); });
    tauri.event.listen('settings', ({ payload }) => pickMode(payload));
    const Win = tauri.window.Window, drag = Win?.prototype.startDragging;
    if (drag) Win.prototype.startDragging = function () { return full ? Promise.resolve() : drag.call(this); };
    addEventListener('keydown', e => {
      const a = HudKeys.action(e, mac, full, panel || !!(document.querySelector('#shop.open, .menu:not([hidden])') || root.classList.contains('dw-open')));
      if (!a || (panel && !('fullscreen' in a))) return;   // the panel: only the fullscreen toggle (its Esc closes its own boxes)
      e.preventDefault();
      const done = 'fullscreen' in a ? tauri.core.invoke('fullscreen', { on: a.fullscreen }) : tauri.core.invoke('step_size', a);
      done.catch(err => console.error('keys', err));
    }, true);
    addEventListener('keydown', e => {
      if (e.repeat || e.metaKey || e.ctrlKey || e.altKey || e.target.closest?.('input, textarea, [contenteditable]')) return;
      const k = e.key.toLowerCase();
      if (k === 'p') { tauri.core.invoke('photo'); return; }   // photo mode: no game key is P
      if (k === ' ') { e.target.blur?.(); e.preventDefault(); }   // the answer comes back too late to decide
      call('key', { k });
    });

    // the app steps the game at the rate widget.js picks (awake, the frame-rate setting; 0 while hidden)
    let fps = st.fps;
    Object.defineProperty(st, 'fps', { get: () => fps, set: v => { if (v !== fps && !panel) call('fps', { fps: fps = v }); }, configurable: true });   // the game's window sets the pace

    // the pictures game.js hands the page: hero sprites ({w, h, col}, as game.js's) and the frame for the share card
    const sprites = {};
    const sprite = id => sprites[id] || (sprites[id] = call('sprite', { id }).then(JSON.parse));
    const thumb = (id, k = 4) => {
      const c = document.createElement('canvas');
      sprite(id).then(spr => {
        c.width = spr.w * k; c.height = spr.h * k; const x = c.getContext('2d');
        for (let j = 0; j < spr.h; j++) for (let i = 0; i < spr.w; i++) { const v = spr.col[j * spr.w + i]; if (v < 0) continue; x.fillStyle = '#' + v.toString(16).padStart(6, '0'); x.fillRect(i * k, j * k, k, k); }
      }).catch(e => console.error(e));
      return c;
    };
    // the card is drawn as soon as share is clicked: on the layer, its frame and sprite are fetched as it's pressed
    const scene = document.createElement('canvas');
    let cardSpr = { w: 0, h: 0, col: [] };
    const forCard = () => {
      call('sprite', {}).then(s => { cardSpr = JSON.parse(s); }).catch(e => console.error(e));
      const put = (b, w, h) => { if (b.byteLength !== w * h * 4) return; scene.width = w; scene.height = h; scene.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(b), w, h), 0, 0); };
      if (native && !panel) call('pixels').then(b => put(b, st.W, st.H)).catch(e => console.error(e));
      // the panel draws no planet: the app gives it the game's frame once, after it asks for the card (control.rs)
      if (panel) call('card').then(() => Promise.all([call('spot').then(JSON.parse), call('pixels')])).then(([s, b]) => put(b, s.w, s.h)).catch(e => console.error(e));
    };
    addEventListener('pointerdown', e => { if (e.target.closest?.('#share, #shareM, [data-w="share"]')) forCard(); }, true);
    forCard();

    const resize = () => { if (panel) return; call('resize', { w: st.W, h: st.H, scale: st.scale }); st.buf.fill(0); show(); };
    // the side drawer slides the planet aside (a transition on the stage): the layer follows it frame by frame
    const relayout = () => { if (panel) return; const until = performance.now() + 450, step = () => { show(); if (performance.now() < until) requestAnimationFrame(step); }; step(); };
    resize();
    call('resend');
    const g = {
      native: true,
      menu: () => view.menu, angels: () => view.angels, ascendInfo: () => view.ascend_info, godInfo: () => view.god_info, zenoInfo: () => view.zeno_info,
      card: () => Object.assign({}, view.card, { spr: cardSpr, scene: native || panel ? scene : st.cv }),
      frame: () => {},
      learn: id => { call('learn', { id }); rerender(); },
      respec: () => { call('respec'); rerender(); },
      learnAngel: id => { call('learn_angel', { id }); rerender(); },
      autoNext: () => { call('auto_next'); rerender(); },
      wear: (kind, id) => { call('wear', { kind, id }); rerender(); },
      pick: id => { call('pick_hero', { id }); rerender(); return true; },
      thumb,
      // the shop's hover card and its prices buying ×10 or max, which page.html takes as promises here
      info: key => call('shop_info', { key }).then(JSON.parse), bulk: n => call('bulk', { n }).then(JSON.parse),
      kaioken: () => call('kaioken'),
      itx: () => call('itx'),
      train: () => call('train'),
      start: () => (panel ? Promise.resolve() : call('start', { today: new Date().toDateString(), now_ms: Date.now() })),   // the game's window starts it
      resize,
      relayout,
    };
    if (panel) {
      ccUi = CONTROL.mount({ game: g, invoke: CC.invoke, settings: CC.settings, fresh: () => fresh });
      // the app sent nothing while it was hidden: everything again once it's back
      document.addEventListener('visibilitychange', () => { if (!document.hidden) call('resend'); });
    }
    return g;
  },
});
