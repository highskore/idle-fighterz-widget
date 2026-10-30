// The first-run card (setting `onboarded`, false only on the very first launch or after "show the intro again"):
// three steps where the dock will be, with the HUD stepped aside meanwhile. 1 rings the planet, 2 marks the window's
// edges and how to drag it, 3 the show/hide key and the menu-bar scouter. Skip or finish, and the app marks it seen.
// Added to the widget's pages by scripts/pages.mjs; it's app UI, in the page's own fonts.
(function (root) {
  // The ring round the planet, in CSS pixels: `spot` is where the core draws it (centre and radius in buffer pixels, and
  // the buffer's size: planet_spot, the camera its hit tests use too), `rect` the canvas it's drawn on (or the layer
  // under it, placed on the same rect). `r` is the drawn disc's, outline included; the ring the same few points clear of it
  const GAP = 9;
  function ring(spot, rect) {
    // the disc is drawn with a one-pixel outline round its radius: the margin is measured from that, at 1× and 2× alike
    const kx = rect.width / spot.w, ky = rect.height / spot.h, r = (spot.r + 1) * (kx + ky) / 2;
    return { x: rect.left + spot.cx * kx, y: rect.top + spot.cy * ky, r, ring: r + GAP };
  }
  // game.js in the page (no core to ask): its camera for the widget, as geom.rs's layout() puts it
  const guess = cv => ({ w: cv.width, h: cv.height, cx: Math.round(cv.width / 2), cy: Math.round(cv.height / 2), r: Math.max(56, Math.min(70, Math.round(Math.min(cv.width, cv.height) * .2))) });
  if (typeof module === 'object' && module.exports) module.exports = { ring, guess, GAP };
  const tauri = root.__TAURI__;
  if (!tauri) return;
  const css = document.createElement('style');
  css.textContent = `
html.ob-on .w, html.ob-on .w-ring { visibility: hidden !important; }
.ob { position: fixed; left: 50%; bottom: 8px; transform: translateX(-50%); z-index: 50; width: min(300px, calc(100vw - 16px)); padding: 11px 13px 10px; font: 12px/1.5 var(--mono, ui-monospace, monospace); color: #0b0b0b; background: rgba(255,255,255,.97); border: 1px solid rgba(0,0,0,.08); border-radius: 16px; box-shadow: 0 8px 26px rgba(0,0,0,.28); }
.ob-top { display: flex; align-items: center; justify-content: space-between; font-size: 9px; letter-spacing: .1em; text-transform: uppercase; color: #8d8d8d; }
.ob-dots { display: flex; gap: 4px; } .ob-dots i { width: 14px; height: 3px; border-radius: 2px; background: #ececec; } .ob-dots i.on { background: #d99a1c; }
.ob h3 { margin: 6px 0 3px; font: italic 22px/1.05 var(--serif, Georgia, serif); font-weight: 400; letter-spacing: -.01em; }
.ob p { margin: 0; font-size: 11px; line-height: 1.45; color: #555; }
.ob kbd { font: inherit; font-size: 10px; padding: 0 4px; border: 1px solid #ddd; border-bottom-width: 2px; border-radius: 4px; background: #fafafa; color: #0b0b0b; }
.ob-act { display: flex; justify-content: space-between; align-items: center; margin-top: 9px; }
.ob-act button { font: inherit; font-size: 11px; text-transform: lowercase; border: 0; border-radius: 999px; padding: 5px 13px; cursor: pointer; }
.ob-act .skip { padding-left: 0; color: #8d8d8d; background: none; }
.ob-act .next { color: #fff; background: #0b0b0b; }
.ob-ring { position: fixed; z-index: 49; width: 176px; height: 176px; margin: -88px 0 0 -88px; border-radius: 50%; border: 2px dashed #d99a1c; box-shadow: 0 0 0 4px rgba(255,255,255,.55), inset 0 0 0 4px rgba(255,255,255,.55); pointer-events: none; }
.ob-tag { position: fixed; z-index: 49; transform: translateX(-50%); padding: 1px 8px; font: 11px/1.5 var(--mono, ui-monospace, monospace); color: #0b0b0b; background: rgba(255,255,255,.95); border-radius: 999px; box-shadow: 0 1px 4px rgba(0,0,0,.25); pointer-events: none; white-space: nowrap; }
@keyframes obp { 50% { transform: translate(-3px, -4px); } }
.ob-ring { box-sizing: border-box; }
.ob.ob-up { top: 8px; bottom: auto; }
@media (prefers-reduced-motion: reduce) { .ob-tap { animation: none; } }
.ob-act .ob-dots { display: none; }
/* a small widget: a shorter card, its step dots between the buttons, so the planet above stays in sight */
@media (max-height: 380px) { .ob { padding: 7px 11px 6px; } .ob-top { display: none; } .ob-act .ob-dots { display: flex; } .ob h3 { margin: 0 0 2px; font-size: 16px; } .ob p { font-size: 10px; line-height: 1.3; } .ob-act { margin-top: 4px; } .ob-act button { padding: 2px 11px; } }
.ob-tap { position: fixed; z-index: 49; width: 30px; height: 30px; pointer-events: none; animation: obp 1.2s ease-in-out infinite; filter: drop-shadow(0 2px 3px rgba(0,0,0,.4)); }
.ob-win { position: fixed; inset: 4px; z-index: 49; border: 2px dashed rgba(217,154,28,.95); border-radius: 14px; box-shadow: 0 0 0 2px rgba(255,255,255,.6), inset 0 0 0 2px rgba(255,255,255,.6); pointer-events: none; }
.ob-arrow { position: fixed; z-index: 49; width: 26px; height: 26px; display: grid; place-items: center; font: 14px/1 system-ui, sans-serif; color: #0b0b0b; background: rgba(255,255,255,.95); border-radius: 50%; box-shadow: 0 2px 6px rgba(0,0,0,.3); pointer-events: none; }`;
  document.head.append(css);

  const HAND = '<svg class="ob-tap" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 11V4.5a1.5 1.5 0 0 1 3 0V10l5.2 1.1c1 .2 1.8 1.2 1.6 2.3l-.9 5.2A2 2 0 0 1 15.9 20h-5.3a2 2 0 0 1-1.6-.8L5.4 14.6a1.5 1.5 0 0 1 2.3-1.9z" fill="#fff" stroke="#111" stroke-width="1.4" stroke-linejoin="round"/></svg>';
  const el = html => { const t = document.createElement('template'); t.innerHTML = html.trim(); return t.content.firstChild; };
  const esc = t => String(t).replace(/[&<>"']/g, c => '&#' + c.charCodeAt(0) + ';');
  let s = {}, step = 0, parts = [], follow = 0;

  const keys = () => {
    const mac = s.os === 'macos', sc = s.shortcut || '';
    const name = p => (mac ? { Ctrl: '⌃', Alt: '⌥', Shift: '⇧', Cmd: '⌘' }[p] : p === 'Cmd' ? 'Super' : p) || p;
    return sc ? sc.split('+').map(p => `<kbd>${esc(name(p).replace(/^(Key|Digit)/, ''))}</kbd>`).join(mac ? '' : '+') : '';
  };
  const STEPS = () => {
    const mac = s.os === 'macos', alt = mac ? '⌥' : 'alt', bar = mac ? 'menu bar' : 'tray', k = keys();
    return [
      ['your planet lives here now', 'tap the planet to train. power keeps climbing while you work, and faster when you play.'],
      ['move it anywhere', `drag the empty sky to move the widget, or hold <kbd>${alt}</kbd> and drag from anywhere.`],
      ['hide and find it', `${k ? k + ' hides it and brings it back. ' : ''}the scouter in your ${bar} shows your power, and has settings.`],
    ];
  };
  const clear = () => { clearInterval(follow); for (const p of parts) p.remove(); parts = []; };
  const add = (e, x, y) => { if (x !== undefined) { e.style.left = x + 'px'; e.style.top = y + 'px'; } document.body.append(e); parts.push(e); return e; };

  function show(n) {
    clear(); step = n;
    document.documentElement.classList.add('ob-on');
    const [h, p] = STEPS()[n - 1];
    const card = add(el(`<div class="ob" role="dialog" aria-label="welcome"><div class="ob-top"><span>welcome · ${n} of 3</span><span class="ob-dots">${[1, 2, 3].map(k => `<i class="${k <= n ? 'on' : ''}"></i>`).join('')}</span></div><h3>${h}</h3><p>${p}</p><div class="ob-act"><button type="button" class="skip">skip</button><span class="ob-dots">${[1, 2, 3].map(k => `<i class="${k <= n ? 'on' : ''}"></i>`).join('')}</span><button type="button" class="next">${n < 3 ? 'next' : 'start playing'}</button></div></div>`));
    card.querySelector('.skip').addEventListener('click', done);
    card.querySelector('.next').addEventListener('click', () => (n < 3 ? show(n + 1) : done()));
    if (n === 1) {
      const parts1 = [add(el('<div class="ob-ring"></div>')), add(el(HAND)), add(el('<span class="ob-tag">tap to train</span>'))];
      place(parts1);
      follow = setInterval(() => place(parts1), 250);
    } else if (n === 2) {
      add(el('<div class="ob-win"></div>'));
      const w = innerWidth, hh = innerHeight;
      for (const [x, y, a] of [[18, 18, '↖'], [w - 44, 18, '↗'], [18, hh * .45, '←'], [w - 44, hh * .45, '→']]) add(el(`<span class="ob-arrow">${a}</span>`), x, y);
      add(el(HAND), 54, 56);
    }
  }
  // step 1's ring, hand and label, where the planet is now: asked of the core a few times a second while it shows, so
  // they follow a resize, a new layout or skin, and the shake (a drag or spin turns the planet in place)
  function place([e, hand, tag]) {
    const cv = document.querySelector('.stage canvas');
    if (!cv) return;
    tauri.core.invoke('planet_spot').then(JSON.parse, () => guess(cv)).then(spot => {
      if (!e.isConnected) return;
      const p = ring(spot, cv.getBoundingClientRect()), d = 2 * p.ring;
      Object.assign(e.style, { left: p.x + 'px', top: p.y + 'px', width: d + 'px', height: d + 'px', margin: `${-p.ring}px 0 0 ${-p.ring}px` });
      Object.assign(hand.style, { left: p.x + p.r * .45 + 'px', top: p.y + p.r * .4 + 'px' });
      // the card goes on the side of the planet with more room (above it when a top strip pushes the planet down)
      const up = p.y - p.ring > innerHeight - p.y - p.ring + 10, card = document.querySelector('.ob');
      if (card) card.classList.toggle('ob-up', up);
      Object.assign(tag.style, { left: p.x + 'px', top: (up ? Math.min(innerHeight - 30, p.y + p.ring + 6) : Math.max(4, p.y - p.ring - 26)) + 'px' });
    });
  }
  function done() {
    clear(); step = 0;
    document.documentElement.classList.remove('ob-on');
    tauri.core.invoke('intro_done').catch(() => {});
  }
  addEventListener('keydown', e => { if (step && e.key === 'Escape') done(); }, true);
  addEventListener('resize', () => { if (step) show(step); });

  const apply = v => {
    s = v || {};
    if (s.onboarded === false && !step) show(1);
    else if (s.onboarded === true && step) { clear(); step = 0; document.documentElement.classList.remove('ob-on'); }
  };
  // after the widget has laid itself out, so the planet is where it'll stay
  const ready = () => tauri.core.invoke('get_settings').then(apply);
  if (document.readyState === 'complete') setTimeout(ready, 300); else addEventListener('load', () => setTimeout(ready, 300));
  tauri.event.listen('settings', e => apply(e.payload));
})(typeof window === 'object' ? window : globalThis);
