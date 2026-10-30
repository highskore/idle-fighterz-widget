// The first-run card (setting `onboarded`, false only on the very first launch or after "show the intro again"):
// three steps where the dock will be, with the HUD stepped aside meanwhile. 1 rings the planet, 2 marks the window's
// edges and how to drag it, 3 the show/hide key and the menu-bar scouter. Skip or finish, and the app marks it seen.
// Added to the widget's pages by scripts/pages.mjs; it's app UI, in the page's own fonts.
(function () {
  const tauri = window.__TAURI__;
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
.ob-ring { position: fixed; z-index: 49; width: 176px; height: 176px; margin: -88px 0 0 -88px; border-radius: 50%; border: 2px dashed #d99a1c; box-shadow: 0 0 0 4px rgba(255,255,255,.55), inset 0 0 0 4px rgba(255,255,255,.55); pointer-events: none; animation: obp 1.6s ease-in-out infinite; }
.ob-tag { position: fixed; z-index: 49; transform: translateX(-50%); padding: 1px 8px; font: 11px/1.5 var(--mono, ui-monospace, monospace); color: #0b0b0b; background: rgba(255,255,255,.95); border-radius: 999px; box-shadow: 0 1px 4px rgba(0,0,0,.25); pointer-events: none; white-space: nowrap; }
@keyframes obp { 50% { transform: scale(1.05); } }
@media (prefers-reduced-motion: reduce) { .ob-ring { animation: none; } }
.ob-tap { position: fixed; z-index: 49; width: 30px; height: 30px; pointer-events: none; filter: drop-shadow(0 2px 3px rgba(0,0,0,.4)); }
.ob-win { position: fixed; inset: 4px; z-index: 49; border: 2px dashed rgba(217,154,28,.95); border-radius: 14px; box-shadow: 0 0 0 2px rgba(255,255,255,.6), inset 0 0 0 2px rgba(255,255,255,.6); pointer-events: none; }
.ob-arrow { position: fixed; z-index: 49; width: 26px; height: 26px; display: grid; place-items: center; font: 14px/1 system-ui, sans-serif; color: #0b0b0b; background: rgba(255,255,255,.95); border-radius: 50%; box-shadow: 0 2px 6px rgba(0,0,0,.3); pointer-events: none; }`;
  document.head.append(css);

  const HAND = '<svg class="ob-tap" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 11V4.5a1.5 1.5 0 0 1 3 0V10l5.2 1.1c1 .2 1.8 1.2 1.6 2.3l-.9 5.2A2 2 0 0 1 15.9 20h-5.3a2 2 0 0 1-1.6-.8L5.4 14.6a1.5 1.5 0 0 1 2.3-1.9z" fill="#fff" stroke="#111" stroke-width="1.4" stroke-linejoin="round"/></svg>';
  const el = html => { const t = document.createElement('template'); t.innerHTML = html.trim(); return t.content.firstChild; };
  const esc = t => String(t).replace(/[&<>"']/g, c => '&#' + c.charCodeAt(0) + ';');
  let s = {}, step = 0, parts = [];

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
  const clear = () => { for (const p of parts) p.remove(); parts = []; };
  const add = (e, x, y) => { if (x !== undefined) { e.style.left = x + 'px'; e.style.top = y + 'px'; } document.body.append(e); parts.push(e); return e; };

  function show(n) {
    clear(); step = n;
    document.documentElement.classList.add('ob-on');
    const [h, p] = STEPS()[n - 1];
    const card = add(el(`<div class="ob" role="dialog" aria-label="welcome"><div class="ob-top"><span>welcome · ${n} of 3</span><span class="ob-dots">${[1, 2, 3].map(k => `<i class="${k <= n ? 'on' : ''}"></i>`).join('')}</span></div><h3>${h}</h3><p>${p}</p><div class="ob-act"><button type="button" class="skip">skip</button><button type="button" class="next">${n < 3 ? 'next' : 'start playing'}</button></div></div>`));
    card.querySelector('.skip').addEventListener('click', done);
    card.querySelector('.next').addEventListener('click', () => (n < 3 ? show(n + 1) : done()));
    const stage = document.querySelector('.stage') || document.body, r = stage.getBoundingClientRect();
    const cx = r.left + r.width / 2, cy = r.top + r.height / 2, ring = Math.min(176, r.width * .5, r.height * .5);
    if (n === 1) {
      const e = add(el('<div class="ob-ring"></div>'), cx, cy); e.style.width = e.style.height = ring + 'px'; e.style.margin = `${-ring / 2}px 0 0 ${-ring / 2}px`;
      add(el(HAND), cx + ring * .17, cy + ring * .15);
      add(el('<span class="ob-tag">tap to train</span>'), cx, cy - ring / 2 - 26);
    } else if (n === 2) {
      add(el('<div class="ob-win"></div>'));
      const w = innerWidth, hh = innerHeight;
      for (const [x, y, a] of [[18, 18, '↖'], [w - 44, 18, '↗'], [18, hh * .45, '←'], [w - 44, hh * .45, '→']]) add(el(`<span class="ob-arrow">${a}</span>`), x, y);
      add(el(HAND), 54, 56);
    }
    card.querySelector('.next').focus({ preventScroll: true });
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
})();
