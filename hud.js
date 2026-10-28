// The planet on the Rust core (cargo feature rust-core): planet-core runs in the app (src-tauri/src/planet.rs), which
// steps it and sends a `planet` event with each frame's message. This hands page.html a `game` object with game.js's
// API over the app's planet_* commands and drives page.html's `ui` from those messages. It's game/rust/planet-web/shim.js
// with `invoke` in place of the wasm `Planet`, and without its lockstep twin: nothing draws the planet yet (NATIVE.md).
GAMES.push({
  id: 'planet',
  mount(st, ui) {
    const tauri = window.__TAURI__, call = (cmd, args) => tauri.core.invoke('planet_' + cmd, args);
    const ptr = e => { const [bx, by] = st.toBuf(e); return { x: e.clientX, y: e.clientY, bx, by }; };
    // nothing draws into the buffer: mark it all as something to press, so widget.js's empty-sky test leaves presses to
    // the game (⌥/⌘-press still drags the window)
    const solid = () => st.buf.fill(0x01000000);
    solid();

    // page.html reads these synchronously, so they're kept from the last time anything changed
    const view = {
      menu: { ach: [], tree: [], branches: [], chars: [], seeds: 0, spent: 0, boost: 1 }, angels: { gods: false, godki: 0, boost: 1, auto: '', list: [] },
      ascend_info: { ok: false, seeds: 0 }, god_info: {}, zeno_info: {}, card: {},
    };
    const refresh = () => Promise.all(Object.keys(view).map(k => call(k).then(s => { view[k] = JSON.parse(s); })))
      .then(() => { for (const n of view.menu.tree) if (n.max === null) n.max = Infinity; });
    // after a menu button: the menu again, once what it changed is in
    const rerender = () => refresh().then(() => { const tab = document.querySelector('[data-mt].on'); if (tab && !document.getElementById('menu').hidden) tab.click(); });

    // planet-core's messages, onto page.html's ui, in order
    let night = false, shen = false, look = '', queue = refresh();
    const deliver = m => {
      if ('world' in m) { ui.hell && ui.hell(false); ui.world && ui.world(m.world); }
      if (m.hud) {
        ui.update(m.hud);
        if (m.hud.night !== night) { night = m.hud.night; ui.night && ui.night(night); }
        if (m.hud.shen !== shen) document.body.classList.toggle('shen', shen = m.hud.shen);
        if (m.hud.look !== look) { document.body.classList.remove('jelly', 'earth'); look = m.hud.look; if (look) document.body.classList.add(look); }
      }
      if (m.shop) ui.shop(m.shop);
      for (const [text, ms] of m.events) ui.event(text, ms);
      for (const [cue, ...args] of m.cues) if (typeof ui[cue] === 'function') ui[cue](...args);
      if (m.wish) ui.wish(m.wish, key => call('pick_wish', { key }));
    };
    tauri.event.listen('planet', ({ payload }) => {
      const m = JSON.parse(payload);
      queue = queue.then(() => (m.hud || m.shop || m.cues.length ? refresh() : null)).then(() => deliver(m)).catch(e => console.error(e));
    });
    ui.onBuy(key => call('buy', { key }));

    st.cv.addEventListener('pointerdown', e => call('pointer_down', ptr(e)));
    st.cv.addEventListener('pointermove', e => call('pointer_move', ptr(e)));
    st.cv.addEventListener('pointerup', e => { const { bx, by } = ptr(e); call('pointer_up', { bx, by }); });
    st.cv.addEventListener('pointercancel', () => call('pointer_cancel'));
    addEventListener('keydown', e => {
      if (e.repeat || e.metaKey || e.ctrlKey || e.altKey || e.target.closest?.('input, textarea, [contenteditable]')) return;
      const k = e.key.toLowerCase();
      if (k === ' ') { e.target.blur?.(); e.preventDefault(); }   // the answer comes back too late to decide
      call('key', { k });
    });

    // the app steps the game at the rate widget.js picks (awake, the frame-rate setting; 0 while hidden)
    let fps = st.fps;
    Object.defineProperty(st, 'fps', { get: () => fps, set: v => { if (v !== fps) call('fps', { fps: fps = v }); }, configurable: true });

    const resize = () => { call('resize', { w: st.W, h: st.H, scale: st.scale }); solid(); };
    resize();
    call('resend');
    const blank = () => document.createElement('canvas');
    return {
      native: true,
      menu: () => view.menu, angels: () => view.angels, ascendInfo: () => view.ascend_info, godInfo: () => view.god_info, zenoInfo: () => view.zeno_info,
      card: () => Object.assign({}, view.card, { spr: blank(), scene: blank() }),
      frame: () => {},
      learn: id => { call('learn', { id }); rerender(); },
      respec: () => { call('respec'); rerender(); },
      learnAngel: id => { call('learn_angel', { id }); rerender(); },
      autoNext: () => { call('auto_next'); rerender(); },
      pick: id => { call('pick_hero', { id }); rerender(); return true; },
      thumb: blank,
      kaioken: () => call('kaioken'),
      train: () => call('train'),
      start: () => call('start', { today: new Date().toDateString(), now_ms: Date.now() }),
      resize,
    };
  },
});
