// node --test test/: the widget's own keys (hud-keys.js)
const test = require('node:test');
const assert = require('node:assert');
const { action } = require('../hud-keys.js');

const key = (code, mods = {}, k = '') => ({ code, key: k, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, repeat: false, ...mods });

test('⌘ +, − and 0 step the UI size on macOS; ctrl elsewhere', () => {
  assert.deepStrictEqual(action(key('Equal', { metaKey: true }), true), { size: 'ui_size', by: 1 });
  assert.deepStrictEqual(action(key('Equal', { metaKey: true, shiftKey: true }), true), { size: 'ui_size', by: 1 }, '⌘⇧= is ⌘+');
  assert.deepStrictEqual(action(key('Minus', { metaKey: true }), true), { size: 'ui_size', by: -1 });
  assert.deepStrictEqual(action(key('Digit0', { metaKey: true }), true), { size: 'ui_size', by: 0 });
  assert.deepStrictEqual(action(key('NumpadAdd', { ctrlKey: true }), false), { size: 'ui_size', by: 1 });
  assert.deepStrictEqual(action(key('Minus', { ctrlKey: true }), false), { size: 'ui_size', by: -1 });
  assert.strictEqual(action(key('Equal', { ctrlKey: true }), true), null, 'ctrl isn\'t ⌘ on macOS');
  assert.strictEqual(action(key('Equal', { metaKey: true }), false), null, 'nor the Windows key ctrl');
  assert.strictEqual(action(key('Equal')), null, 'a bare = is the game\'s');
});

test('⌥/alt +, − and 0 step the planet size', () => {
  assert.deepStrictEqual(action(key('Equal', { altKey: true }, '≠'), true), { size: 'planet_size', by: 1 }, 'by the key\'s place, not what ⌥ types');
  assert.deepStrictEqual(action(key('Minus', { altKey: true }, '–'), true), { size: 'planet_size', by: -1 });
  assert.deepStrictEqual(action(key('NumpadSubtract', { altKey: true }), false), { size: 'planet_size', by: -1 });
  assert.deepStrictEqual(action(key('Digit0', { altKey: true }), false), { size: 'planet_size', by: 0 });
  assert.strictEqual(action(key('Equal', { altKey: true, metaKey: true }), true), null);
});

test('fullscreen: ⌃⌘F on macOS, F11 elsewhere; Esc leaves once no panel is open', () => {
  assert.deepStrictEqual(action(key('KeyF', { metaKey: true, ctrlKey: true }, 'f'), true, false), { fullscreen: true });
  assert.deepStrictEqual(action(key('KeyF', { metaKey: true, ctrlKey: true }, 'f'), true, true), { fullscreen: false });
  assert.strictEqual(action(key('F11', {}, 'F11'), true, false), null, 'F11 shows the desktop on macOS');
  assert.deepStrictEqual(action(key('F11', {}, 'F11'), false, false), { fullscreen: true });
  assert.deepStrictEqual(action(key('Escape', {}, 'Escape'), false, true, false), { fullscreen: false });
  assert.strictEqual(action(key('Escape', {}, 'Escape'), false, true, true), null, 'a panel open: its Esc first');
  assert.strictEqual(action(key('Escape', {}, 'Escape'), false, false, false), null, 'not fullscreen');
  assert.strictEqual(action(key('KeyF', { metaKey: true, ctrlKey: true, repeat: true }, 'f'), true, false), null, 'held');
});
