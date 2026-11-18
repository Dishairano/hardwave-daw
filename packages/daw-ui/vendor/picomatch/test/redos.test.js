'use strict';

// GHSA-fc5q-39xc-vjpf: a repeated extglob with one branch that holds an open
// quantifier, such as `+(*)` or `+(*a)`, compiled to a regex that backtracks
// exponentially on a short string that almost matches. picomatch 4.0.7 takes
// seconds on each 25-character string below, twice as long for every
// character more; this fork answers at once.
//
//   node --test test/                                     this fork: passes
//   PICOMATCH=<path to picomatch 4.0.7> node --test test/  upstream: fails

const { test } = require('node:test');
const assert = require('node:assert');
const path = require('node:path');

const picomatch = require(process.env.PICOMATCH ? path.resolve(process.env.PICOMATCH) : '..');

const BUDGET_MS = 100;

const slowUpstream = [
  ['+(*)', 'a'.repeat(24) + '/'],
  ['*(*)', 'a'.repeat(24) + '/'],
  ['+(*a)', 'a'.repeat(24) + 'b'],
  ['*(*a)', 'a'.repeat(24) + 'b'],
  ['+(a*)', 'a'.repeat(24) + '/'],
  ['+(*?)', 'a'.repeat(24) + '/'],
  ['+(!(b))', 'a'.repeat(24) + '/'],
  ['+(@(*a))', 'a'.repeat(24) + 'b'],
  ['+(a+)', 'a'.repeat(24) + 'b'],
  ['+(*[a-z])', 'a'.repeat(24) + '/'],
  ['+([a-z]*)', 'a'.repeat(24) + '/'],
  // A repeated extglob nested in the one branch.
  ['+(*(a)a)', 'a'.repeat(25) + 'b'],
  ['*(*(a)a)', 'a'.repeat(25) + 'b'],
  ['+(a*(a))', 'a'.repeat(25) + 'b'],
  ['+(*(a|b)a)', 'a'.repeat(25) + 'b'],
  ['+(+(b)?(a))', 'b'.repeat(25) + '/'],
  ['+(@(a|b)@(a|b)?)', 'a'.repeat(25) + '!'],
  // Several branches that each start with `*`.
  ['+(*a|*a)', 'a'.repeat(18) + 'b'],
  ['+(*.js|*.ts)', '.js'.repeat(24) + '/']
];

for (const [glob, input] of slowUpstream) {
  test(`${glob} answers at once on a string that almost matches`, () => {
    const re = picomatch.makeRe(glob);
    const start = process.hrtime.bigint();
    re.test(input);
    const ms = Number(process.hrtime.bigint() - start) / 1e6;
    assert.ok(ms < BUDGET_MS, `${glob} took ${ms.toFixed(0)} ms on ${input.length} characters`);
  });
}

// Where repeating the branch cannot match more than the branch alone, the
// fork drops the repetition and keeps the meaning. With the safeguard off
// (`maxExtglobRecursion: false`) the pattern compiles exactly as upstream
// does, so the two must agree on every short string.
const collapsed = [
  '+(*)', '*(*)', '+(*a)', '*(*a)', '+(a*)', '*(a*)', '+(*?)', '+(?*)', '+(*.js)',
  '+(a*b)', '*(*a*)', '+(*a+)', '+(!(b))', '*(!(foo))', '+(!(f))', 'x+(*a)', '+(*)c',
  '+(*)x', 'a/+(*b)', '+(*a)/b', '*+(a*)', '*(!(a))b',
  // Several branches, all with a leading or all with a trailing `*`.
  '+(*a|*a)', '+(*.js|*.ts)', '+(a*|b*)', '*(*a|*b)', 'x+(*a|*bc)'
];

const strings = [''];
for (let len = 1; len <= 5; len++) {
  for (const s of strings.filter(s => s.length === len - 1)) {
    for (const ch of ['a', 'b', 'c', '.', '/', 'x', '\\']) strings.push(s + ch);
  }
}

// Repeated extglobs that cannot backtrack exponentially are compiled as
// upstream compiles them. With `windows: true` a backslash is a separator
// that a `*` does not cross, so `+(*\\a)` is kept there; without it, it is
// dropped to one repetition, which matches the same strings.
const kept = ['*(*(of*(o)x)o)', '+(a+b)', '+(*/)', '+(abc)', '*(foo|bar)', '+(*\\\\a)', '+(a\\\\*)', '*(*\\\\)'];

for (const glob of [...collapsed, ...kept]) {
  test(`${glob} matches exactly what it matched before`, () => {
    for (const options of [{}, { dot: true }, { windows: true }, { nocase: true }, { capture: true }]) {
      const fixed = picomatch.makeRe(glob, options);
      const original = picomatch.makeRe(glob, { ...options, maxExtglobRecursion: false });
      for (const s of strings) {
        assert.strictEqual(fixed.test(s), original.test(s), `${glob} ${JSON.stringify(options)} on ${JSON.stringify(s)}`);
      }
    }
  });
}

test('the patterns picomatch documents keep working', () => {
  assert.ok(picomatch.isMatch('abc', '+(*)c'));
  assert.ok(!picomatch.isMatch('abc', '+(*)x'));
  assert.ok(picomatch.isMatch('banana', '+(*a)'));
  assert.ok(!picomatch.isMatch('bananas', '+(*a)'));
  assert.ok(picomatch.isMatch('fff', '+(!(f))'));
  assert.ok(picomatch.isMatch('bar', '*(!(foo))'));
  assert.ok(!picomatch.isMatch('foo', '*(!(foo))'));
  assert.ok(picomatch.isMatch('src/a.test.ts', 'src/**/*.test.{ts,tsx}'));
  assert.ok(picomatch.isMatch('ofoxo', '*(*(of*(o)x)o)'));
  assert.ok(picomatch.isMatch('\\a\\a', '+(*\\\\a)', { windows: false }));
  assert.ok(picomatch.makeRe('+(*\\\\a)', { windows: true }).test('\\a\\a'));
});

test('a nested repeated extglob that would backtrack is matched literally', () => {
  for (const glob of ['+(*(a)a)', '*(*(a)a)', '+(a*(a))', '+(*(a|b)a)']) {
    assert.ok(picomatch.isMatch(glob, glob), glob);
    assert.ok(!picomatch.isMatch('aaa', glob), glob);
  }
});
