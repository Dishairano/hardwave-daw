'use strict';

// Differential fuzz: random globs compiled by this fork and by picomatch
// 4.0.7 (`hardwaveFork: false` compiles exactly as 4.0.7 does). Where the two
// regexes differ, the fork must
// - match exactly the same strings (all strings up to four characters),
//   unless it fell back to matching a repeated extglob literally, which is
//   upstream's own treatment of a risky one; and
// - never be slower: on hostile strings it may take at most 3x upstream's
//   time plus a 20 ms allowance.
//
//   node fuzz/differential.js [count] [seed] [length]   (a larger run)

const picomatch = require('..');

const ATOMS = ['a', 'b', '.', '/', '\\a', '\\\\', '[ab]', '[a-z]', '?', '*', '**', 'a+', '+', 'x'];
const GROUPS = ['@', '*', '+', '?', '!'];
const OPTIONS = [{}, { windows: true }, { dot: true }, { nocase: true }];
const ALPHABET = ['a', 'b', '.', '/', '\\', 'x'];

const random = seed => () => {
  seed = (seed * 1103515245 + 12345) % 2147483648;
  return (seed >>> 8) / (2147483648 >>> 8);
};

const generate = (rnd, depth = 0) => {
  let out = '';
  const items = 1 + Math.floor(rnd() * 3);
  for (let i = 0; i < items; i++) {
    if (depth < 3 && rnd() < 0.35) {
      const type = GROUPS[Math.floor(rnd() * GROUPS.length)];
      const branches = [];
      const count = 1 + Math.floor(rnd() * (rnd() < 0.6 ? 1 : 3));
      for (let b = 0; b < count; b++) branches.push(generate(rnd, depth + 1));
      out += `${type}(${branches.join('|')})`;
    } else {
      out += ATOMS[Math.floor(rnd() * ATOMS.length)];
    }
  }
  return out;
};

const shortStrings = (() => {
  const all = [''];
  for (let len = 1; len <= 4; len++) {
    for (const s of all.filter(x => x.length === len - 1)) {
      for (const ch of ALPHABET) all.push(s + ch);
    }
  }
  return all;
})();

const hostile = length => {
  const out = [];
  for (const unit of ['a', 'b', 'ab', 'a.', 'ba', 'a/', '\\a']) {
    for (const end of ['!', '/', 'c', '\\']) {
      out.push(unit.repeat(Math.ceil(length / unit.length)).slice(0, length) + end);
    }
  }
  return out;
};

const timed = (re, s) => {
  const start = process.hrtime.bigint();
  re.test(s);
  return Number(process.hrtime.bigint() - start) / 1e6;
};

const compile = (glob, options) => {
  try {
    return picomatch.makeRe(glob, options);
  } catch (err) {
    return err;
  }
};

const run = ({ count, seed, length }) => {
  const rnd = random(seed);
  const stats = { patterns: 0, same: 0, collapsedCompared: 0, literal: 0, mismatches: [], slower: [] };
  const strings = hostile(length);

  for (let i = 0; i < count; i++) {
    const glob = generate(rnd);
    for (const options of OPTIONS) {
      stats.patterns++;
      const fork = compile(glob, options);
      const upstream = compile(glob, { ...options, hardwaveFork: false });
      if (fork instanceof Error || upstream instanceof Error) {
        if (String(fork) !== String(upstream)) stats.mismatches.push([glob, options, 'throws differently']);
        else stats.same++;
        continue;
      }
      if (fork.source === upstream.source) {
        stats.same++;
        continue;
      }

      const actions = picomatch.parse(glob, options).hardwaveForkActions || [];
      if (actions.includes('literal')) {
        stats.literal++;
      } else {
        stats.collapsedCompared++;
        const bad = shortStrings.find(s => fork.test(s) !== upstream.test(s));
        if (bad !== undefined) stats.mismatches.push([glob, options, bad]);
      }

      for (const s of strings) {
        const f = Math.min(timed(fork, s), timed(fork, s));
        if (f <= 20) continue;
        const u = timed(upstream, s);
        if (f > 3 * u + 20) stats.slower.push([glob, options, s, f.toFixed(1), u.toFixed(1)]);
      }
    }
  }
  return stats;
};

module.exports = { run, generate, random };

if (require.main === module) {
  const [count = 20000, seed = 1, length = 22] = process.argv.slice(2).map(Number);
  const start = Date.now();
  const stats = run({ count, seed, length });
  console.log(JSON.stringify({
    globs: count, seed, length, compiled: stats.patterns, identical: stats.same,
    collapsedAndCompared: stats.collapsedCompared, literal: stats.literal,
    mismatches: stats.mismatches.length, slower: stats.slower.length, seconds: (Date.now() - start) / 1000
  }));
  for (const m of stats.mismatches.slice(0, 10)) console.log('MISMATCH', JSON.stringify(m));
  for (const m of stats.slower.slice(0, 10)) console.log('SLOWER', JSON.stringify(m));
  process.exitCode = stats.mismatches.length || stats.slower.length ? 1 : 0;
}
