// The control centre's page logic (control.js): which page shows each toast and cue, the settings each page takes, and
// what each layout holds. Run: node --test test/control.test.cjs
const test = require('node:test');
const assert = require('node:assert/strict');
const C = require('../control.js');

const CUES = ['sfx', 'fx', 'bump', 'achieved', 'zeno', 'treeGift', 'wishDone', 'wish', 'itxPick', 'notify', 'xform', 'quake'];

test('each toast and cue shows on exactly one page, in every layout', () => {
  for (const layout of C.LAYOUTS) {
    for (const what of ['toast', ...CUES]) {
      const on = ['game', 'panel'].filter(who => C.shows(who, layout, what));
      if (what === 'achieved') assert.deepEqual(on, ['game', 'panel'], 'an achievement recounts on both');
      else assert.equal(on.length, 1, `${what} in layout ${layout}: ${on}`);
    }
  }
});

test('toasts stay on the game screen but in layout 2, and the cues that want an answer go to the panel', () => {
  assert.deepEqual(C.LAYOUTS.map(l => C.shows('panel', l, 'toast')), [false, true, false, false, false]);
  for (const cue of ['wish', 'itxPick', 'wishDone', 'zeno', 'treeGift']) assert.ok(C.shows('panel', 1, cue) && !C.shows('game', 1, cue), cue);
  for (const cue of ['sfx', 'notify', 'quake', 'bump']) assert.ok(C.shows('game', 3, cue) && !C.shows('panel', 3, cue), cue);
  for (const what of ['toast', ...CUES]) assert.ok(C.shows('solo', 2, what), 'no control centre: the widget shows everything');
});

test('which page is which', () => {
  assert.equal(C.role(true, { control_on: true }), 'panel');
  assert.equal(C.role(false, { control_on: true }), 'game');
  assert.equal(C.role(false, { control_on: false }), 'solo');
  assert.equal(C.role(false, null), 'solo');
});

test('each page takes the settings its way', () => {
  const s = { skin: 'scouter', layout: 3, hud: 'hover', mode: 'window', opacity: 0.6, clouds: 'always', control_on: true, theme: 'dark' };
  assert.equal(C.lens(s, 'solo'), s, 'untouched');
  const panel = C.lens({ ...s, control_layout: 4 }, 'panel');
  assert.deepEqual([panel.skin, panel.layout, panel.hud, panel.opacity, panel.clouds, panel.theme], ['drawer', 7, 'always', 1, 'never', 'dark']);
  for (const l of [1, 2, 3, 4]) assert.equal(C.lens({ ...s, control_layout: l }, 'game').skin, 'drawer', `layout ${l}: the power chip`);
  const five = C.lens({ ...s, control_layout: 5 }, 'game');
  assert.deepEqual([five.skin, five.layout, five.hud], ['classic', 7, 'always'], 'layout 5: the classic dock');
  assert.equal(s.skin, 'scouter', 'the settings themselves are left as they were');
});

test('every layout reaches upgrades, the ki tree, heroes and achievements', () => {
  for (const l of C.LAYOUTS) {
    const p = C.parts(l), has = k => p.includes(k);
    assert.ok(has('up') || has('shop') || has('deck'), `${l}: upgrades`);
    assert.ok(has('chip') || has('top'), `${l}: the power`);
    // the deck keeps the menus behind its last tile
    if (l !== 4) for (const k of ['tree', 'heroes']) assert.ok(has(k), `${l}: ${k}`);
  }
  assert.ok(C.parts(1).includes('ach') && C.parts(3).includes('ach') && C.parts(5).includes('ach'));
  assert.ok(C.parts(2).includes('toast') && !C.parts(1).includes('toast'), 'the panel has a toast only where it shows them');
  assert.equal(C.pick(9), 1); assert.equal(C.pick('3'), 3);
});

test('the deck has a tile for every upgrade, between the boost and the rest', () => {
  const tiles = C.deck([{ key: 'boots' }, { key: 'gravity' }, { key: 'ascend' }], true);
  assert.deepEqual(tiles.map(t => t.key || t.kind), ['kaio', 'boots', 'gravity', 'ascend', 'itx', 'more']);
  assert.deepEqual(C.deck([], false).map(t => t.kind), ['kaio', 'more']);
});

test('a boost is named as the game names it', () => {
  assert.equal(C.label('', 'kaioken 12s'), 'kaioken', 'no title: the text without its countdown');
  assert.equal(C.label('', 'transmission 1:05'), 'transmission');
  assert.equal(C.label('wolf fang fist', 'wolf fang 3s'), 'wolf fang fist', 'the title, the hero\'s own name');
  assert.equal(C.label(null, 'kaioken'), 'kaioken');
});

test('the tabs count what there is to do', () => {
  const menu = { tree: [{ can: true }, { can: false }, { can: true }], chars: [{ open: true }, { open: false }], ach: [{ done: true }, { done: false }, { done: false }] };
  assert.deepEqual(C.counts(menu, 9), { up: 9, tree: 2, heroes: '1/2', ach: '1/3' });
  assert.deepEqual(C.counts({}, 0), { up: '', tree: '', heroes: '', ach: '' });
});
