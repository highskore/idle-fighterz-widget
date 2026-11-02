// The widget's own keys on the HUD page (hud.js): fullscreen, and the UI and planet sizes of window mode. Put in front of
// hud.js in dist/hud.html by scripts/hud.mjs; node --test test/hud-keys.test.cjs runs it too.
(function (root) {
  const STEP = { Equal: 1, NumpadAdd: 1, Minus: -1, NumpadSubtract: -1, Digit0: 0, Numpad0: 0 };
  // `e` a keydown; `mac` for ⌘ where others have ctrl; `full` whether fullscreen is on; `panel` whether the page has a
  // panel or box open (its own Esc closes that first). What the key asks for: { fullscreen } on or off, { size, by } a
  // step along the UI size (⌘/ctrl +, −, 0) or the planet size (⌥/alt +, −, 0), 0 back to medium; or null
  function action(e, mac, full, panel) {
    if (e.repeat) return null;
    const plain = !e.metaKey && !e.ctrlKey && !e.altKey;
    if (mac ? e.code === 'KeyF' && e.metaKey && e.ctrlKey && !e.altKey && !e.shiftKey : e.key === 'F11' && plain) return { fullscreen: !full };
    if (e.key === 'Escape' && plain && full && !panel) return { fullscreen: false };
    const by = STEP[e.code];
    if (by === undefined) return null;
    const cmd = mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
    if (cmd && !e.altKey) return { size: 'ui_size', by };
    if (e.altKey && !e.metaKey && !e.ctrlKey) return { size: 'planet_size', by };
    return null;
  }
  root.HudKeys = { action };
  if (typeof module === 'object' && module.exports) module.exports = root.HudKeys;
})(typeof window === 'object' ? window : globalThis);
