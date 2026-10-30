// node --test test/: the first-run card's ring, mapped from where the core draws the planet onto the page
const test = require('node:test');
const assert = require('node:assert');
const { ring, guess, GAP } = require('../onboard.js');

// world.js's widget canvas: 1× below 520 points, 2× from there; its toBuf maps the page back onto the buffer
const canvas = size => { const scale = size >= 520 ? 2 : 1, W = Math.ceil(size / scale); return { W, H: W, rect: { left: 0, top: 0, width: size, height: size } }; };
const toBuf = (x, y, c) => [(x - c.rect.left) / c.rect.width * c.W, (y - c.rect.top) / c.rect.height * c.H];

test('the ring sits on the planet at every widget size', () => {
  for (let size = 300; size <= 720; size += 2) {
    const c = canvas(size), spot = { w: c.W, h: c.H, cx: Math.round(c.W / 2) + 3, cy: Math.round(c.H / 2) - 2, r: 64 };
    const p = ring(spot, c.rect), [bx, by] = toBuf(p.x, p.y, c), [ex] = toBuf(p.x + p.r, p.y, c);
    assert.ok(Math.abs(bx - spot.cx) < 1e-9 && Math.abs(by - spot.cy) < 1e-9, `${size}: centre`);
    assert.ok(Math.abs(ex - bx - (spot.r + 1)) < 1e-9, `${size}: the disc's radius, its outline pixel included`);
    assert.strictEqual(p.ring - p.r, GAP, `${size}: the same margin at every size`);
  }
});

test('a canvas placed off the window corner (window mode, a skin) moves the ring with it', () => {
  const spot = { w: 200, h: 200, cx: 100, cy: 100, r: 60 };
  const p = ring(spot, { left: 10, top: 30, width: 400, height: 400 });
  assert.deepStrictEqual([p.x, p.y, p.r], [210, 230, 122]);
});

test("without the core, game.js's widget camera", () => {
  assert.deepStrictEqual(guess({ width: 400, height: 400 }), { w: 400, h: 400, cx: 200, cy: 200, r: 70 });
  assert.deepStrictEqual(guess({ width: 300, height: 300 }), { w: 300, h: 300, cx: 150, cy: 150, r: 60 });
  assert.strictEqual(guess({ width: 260, height: 260 }).r, 56);
});
