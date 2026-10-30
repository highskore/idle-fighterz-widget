// node --test test/: the settings window's search filter
const test = require('node:test');
const assert = require('node:assert');
const { matches } = require('../settings-search.js');

test('an empty search keeps every row', () => {
  assert.ok(matches('', 'opacity', 'opacity', 'widget'));
  assert.ok(matches('   ', 'size', 'size', 'widget'));
});

test('labels match by the start of their words, any case, any order', () => {
  assert.ok(matches('opa', 'opacity 100%', 'opacity', 'widget'));
  assert.ok(matches('LAUNCH', 'launch at login', 'login', 'widget'));
  assert.ok(matches('login launch', 'launch at login', 'login', 'widget'));
  assert.ok(!matches('pacity', 'opacity 100%', 'opacity', 'widget'), 'not the middle of a word');
  assert.ok(!matches('opacity size', 'opacity 100%', 'opacity', 'widget'), 'every word must match');
});

test('synonyms find settings by other names', () => {
  assert.ok(matches('transparent', 'opacity 100%', 'opacity', 'widget'));
  assert.ok(matches('see through', 'opacity 100%', 'opacity', 'widget'));
  assert.ok(!matches('transparent', 'ghost mode click-through', 'ghost', 'widget'));
  assert.ok(matches('do not disturb', 'quiet in focus', 'follow_focus', 'notifications'));
  assert.ok(matches('dnd', 'snooze', 'snooze', 'notifications'));
  assert.ok(matches('hotkey', 'show / hide', 'shortcut', 'widget'));
  assert.ok(matches('preset', 'work', 'profile', 'profiles'));
  assert.ok(matches('zoom', 'size 400 · medium', 'size', 'widget'));
  assert.ok(!matches('zoom', 'opacity 100%', 'opacity', 'widget'));
});

test('a section name keeps its rows', () => {
  assert.ok(matches('notif', 'villain raids', 'notify', 'notifications'));
  assert.ok(matches('profiles', 'save current', 'profile', 'profiles'));
  assert.ok(!matches('notif', 'opacity', 'opacity', 'widget'));
});

test('accents and punctuation are ignored', () => {
  assert.ok(matches('click-through', 'ghost mode click-through', 'ghost', 'widget'));
  assert.ok(matches('cafe', 'café', 'x', 'y'));
});

test('the skin and theme rows have their own names', () => {
  assert.ok(matches('dark mode', 'theme', 'theme', 'widget'));
  assert.ok(matches('appearance', 'skin', 'skin', 'widget'));
  assert.ok(matches('capsule corp', 'skin classic scouter capsule drawer', 'skin', 'widget'));
  assert.ok(!matches('dark mode', 'opacity 100%', 'opacity', 'widget'));
});
