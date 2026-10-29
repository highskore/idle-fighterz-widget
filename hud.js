// The planet on the Rust core (cargo feature rust-core): planet-core runs in the app (src-tauri/src/planet.rs), which
// steps it, draws it with planet-render and sends a `planet` event with each frame's message. This hands page.html a
// `game` object with game.js's API over the app's planet_* commands and drives page.html's `ui` from those messages.
// It's game/rust/planet-web/shim.js with `invoke` in place of the wasm `Planet`, and without its lockstep twin. The
// frames go either to a layer the app keeps under this page (macOS, feature native-renderer) or onto this canvas.
GAMES.push({
  id: 'planet',
  mount(st, ui) {
    const tauri = window.__TAURI__, call = (cmd, args) => tauri.core.invoke('planet_' + cmd, args);
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
    st.el.addEventListener('pointerenter', mask); st.el.addEventListener('pointermove', mask);
    tauri.event.listen('pointer', ({ payload }) => { if (payload.inside) mask(); });   // another app active: no pointer moves

    // how the page shows it: floating or window mode, the clouds widget.js picks (they come and go with the pointer),
    // reduced motion, where the canvas sits and the page's opacity. Several changes at once go as one call
    let viewDue = false;
    const show = () => {
      if (viewDue) return; viewDue = true;
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
      for (const [text, ms] of m.events) ui.event(text, ms);
      for (const [cue, ...args] of m.cues) if (back[cue]) ui[cue] && ui[cue](args[0], back[cue]); else if (typeof ui[cue] === 'function') ui[cue](...args);
    };
    tauri.event.listen('planet', ({ payload }) => {
      const m = JSON.parse(payload);
      if (m.px && !native) pixels();
      const stale = m.shop || m.cues.length || 'world' in m || (m.hud && performance.now() - fresh >= 1000);
      queue = queue.then(() => (stale ? refresh() : null)).then(() => deliver(m)).catch(e => console.error(e));
    });
    ui.onBuy(key => call('buy', { key }));

    st.cv.addEventListener('pointerdown', e => call('pointer_down', { ...ptr(e), button: e.button, touch: e.pointerType === 'touch' }));
    st.cv.addEventListener('pointermove', e => call('pointer_move', ptr(e)));
    st.cv.addEventListener('pointerup', e => { const { bx, by, id } = ptr(e); call('pointer_up', { bx, by, id }); });
    st.cv.addEventListener('pointercancel', () => call('pointer_cancel'));
    st.cv.addEventListener('contextmenu', e => { if (itxOwn) e.preventDefault(); });   // right-click is instant transmission's once it's owned
    addEventListener('keydown', e => {
      if (e.repeat || e.metaKey || e.ctrlKey || e.altKey || e.target.closest?.('input, textarea, [contenteditable]')) return;
      const k = e.key.toLowerCase();
      if (k === ' ') { e.target.blur?.(); e.preventDefault(); }   // the answer comes back too late to decide
      call('key', { k });
    });

    // the app steps the game at the rate widget.js picks (awake, the frame-rate setting; 0 while hidden)
    let fps = st.fps;
    Object.defineProperty(st, 'fps', { get: () => fps, set: v => { if (v !== fps) call('fps', { fps: fps = v }); }, configurable: true });

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
      if (native) call('pixels').then(b => { if (b.byteLength !== st.W * st.H * 4) return; scene.width = st.W; scene.height = st.H; scene.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(b), st.W, st.H), 0, 0); }).catch(e => console.error(e));
    };
    addEventListener('pointerdown', e => { if (e.target.closest?.('#share, #shareM, [data-w="share"]')) forCard(); }, true);
    forCard();

    const resize = () => { call('resize', { w: st.W, h: st.H, scale: st.scale }); st.buf.fill(0); show(); };
    resize();
    call('resend');
    return {
      native: true,
      menu: () => view.menu, angels: () => view.angels, ascendInfo: () => view.ascend_info, godInfo: () => view.god_info, zenoInfo: () => view.zeno_info,
      card: () => Object.assign({}, view.card, { spr: cardSpr, scene: native ? scene : st.cv }),
      frame: () => {},
      learn: id => { call('learn', { id }); rerender(); },
      respec: () => { call('respec'); rerender(); },
      learnAngel: id => { call('learn_angel', { id }); rerender(); },
      autoNext: () => { call('auto_next'); rerender(); },
      pick: id => { call('pick_hero', { id }); rerender(); return true; },
      thumb,
      kaioken: () => call('kaioken'),
      itx: () => call('itx'),
      train: () => call('train'),
      start: () => call('start', { today: new Date().toDateString(), now_ms: Date.now() }),
      resize,
    };
  },
});
