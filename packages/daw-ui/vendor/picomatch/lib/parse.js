'use strict';

const constants = require('./constants');
const utils = require('./utils');

/**
 * Constants
 */

const {
  MAX_LENGTH,
  POSIX_REGEX_SOURCE,
  REGEX_NON_SPECIAL_CHARS,
  REGEX_SPECIAL_CHARS_BACKREF,
  REPLACEMENTS
} = constants;

/**
 * Helpers
 */

const expandRange = (args, options) => {
  if (typeof options.expandRange === 'function') {
    return options.expandRange(...args, options);
  }

  args.sort();
  const value = `[${args.join('-')}]`;

  try {
    /* eslint-disable-next-line no-new */
    new RegExp(value);
  } catch (ex) {
    return args.map(v => utils.escapeRegex(v)).join('..');
  }

  return value;
};

/**
 * Create the message for a syntax error
 */

const syntaxError = (type, char) => {
  return `Missing ${type}: "${char}" - use "\\\\${char}" to match literal characters`;
};

const splitTopLevel = input => {
  const parts = [];
  let bracket = 0;
  let paren = 0;
  let quote = 0;
  let value = '';
  let escaped = false;

  for (const ch of input) {
    if (escaped === true) {
      value += ch;
      escaped = false;
      continue;
    }

    if (ch === '\\') {
      value += ch;
      escaped = true;
      continue;
    }

    if (ch === '"') {
      quote = quote === 1 ? 0 : 1;
      value += ch;
      continue;
    }

    if (quote === 0) {
      if (ch === '[') {
        bracket++;
      } else if (ch === ']' && bracket > 0) {
        bracket--;
      } else if (bracket === 0) {
        if (ch === '(') {
          paren++;
        } else if (ch === ')' && paren > 0) {
          paren--;
        } else if (ch === '|' && paren === 0) {
          parts.push(value);
          value = '';
          continue;
        }
      }
    }

    value += ch;
  }

  parts.push(value);
  return parts;
};

const isPlainBranch = branch => {
  let escaped = false;

  for (const ch of branch) {
    if (escaped === true) {
      escaped = false;
      continue;
    }

    if (ch === '\\') {
      escaped = true;
      continue;
    }

    if (/[?*+@!()[\]{}]/.test(ch)) {
      return false;
    }
  }

  return true;
};

const normalizeSimpleBranch = branch => {
  let value = branch.trim();
  let changed = true;

  while (changed === true) {
    changed = false;

    if (/^@\([^\\()[\]{}|]+\)$/.test(value)) {
      value = value.slice(2, -1);
      changed = true;
    }
  }

  if (!isPlainBranch(value)) {
    return;
  }

  return value.replace(/\\(.)/g, '$1');
};

const hasRepeatedCharPrefixOverlap = branches => {
  const values = branches.map(normalizeSimpleBranch).filter(Boolean);

  for (let i = 0; i < values.length; i++) {
    for (let j = i + 1; j < values.length; j++) {
      const a = values[i];
      const b = values[j];
      const char = a[0];

      if (!char || a !== char.repeat(a.length) || b !== char.repeat(b.length)) {
        continue;
      }

      if (a === b || a.startsWith(b) || b.startsWith(a)) {
        return true;
      }
    }
  }

  return false;
};

const parseRepeatedExtglob = (pattern, requireEnd = true) => {
  if ((pattern[0] !== '+' && pattern[0] !== '*') || pattern[1] !== '(') {
    return;
  }

  let bracket = 0;
  let paren = 0;
  let quote = 0;
  let escaped = false;

  for (let i = 1; i < pattern.length; i++) {
    const ch = pattern[i];

    if (escaped === true) {
      escaped = false;
      continue;
    }

    if (ch === '\\') {
      escaped = true;
      continue;
    }

    if (ch === '"') {
      quote = quote === 1 ? 0 : 1;
      continue;
    }

    if (quote === 1) {
      continue;
    }

    if (ch === '[') {
      bracket++;
      continue;
    }

    if (ch === ']' && bracket > 0) {
      bracket--;
      continue;
    }

    if (bracket > 0) {
      continue;
    }

    if (ch === '(') {
      paren++;
      continue;
    }

    if (ch === ')') {
      paren--;

      if (paren === 0) {
        if (requireEnd === true && i !== pattern.length - 1) {
          return;
        }

        return {
          type: pattern[0],
          body: pattern.slice(2, i),
          end: i
        };
      }
    }
  }
};

const buildCharClassStar = chars => {
  const source = chars.length === 1
    ? utils.escapeRegex(chars[0])
    : `[${chars.map(ch => utils.escapeRegex(ch)).join('')}]`;

  return `${source}*`;
};

const getStarExtglobSequenceChars = pattern => {
  let index = 0;
  const chars = [];

  while (index < pattern.length) {
    const match = parseRepeatedExtglob(pattern.slice(index), false);

    if (!match || match.type !== '*') {
      return;
    }

    const branches = splitTopLevel(match.body).map(branch => branch.trim());
    if (branches.length !== 1) {
      return;
    }

    const branch = normalizeSimpleBranch(branches[0]);
    if (!branch || branch.length !== 1) {
      return;
    }

    chars.push(branch);
    index += match.end + 1;
  }

  if (chars.length < 1) {
    return;
  }

  return chars;
};

const repeatedExtglobRecursion = pattern => {
  let depth = 0;
  let value = pattern.trim();
  let match = parseRepeatedExtglob(value);

  while (match) {
    depth++;
    value = match.body.trim();
    match = parseRepeatedExtglob(value);
  }

  return depth;
};

/**
 * Hardwave fork (GHSA-fc5q-39xc-vjpf, CVE-2026-112366).
 *
 * Reads a repeated extglob's body without compiling it, and says what it
 * holds that can match text of any length:
 *
 * - `star`: a plain `*` at the top level of the body (not `**`, not the
 *   start of a nested `*(`).
 * - `unbounded`: anything at any depth that compiles to an open-ended
 *   quantifier: a plain `*`, `**`, a negation `!(`, or a `+` that is a
 *   quantifier rather than the start of a nested `+(`.
 * - `mayMatchSlash`: anything that could let the body match a `/` or that
 *   this reading does not follow: a slash, a bracket, a brace, a quote or a
 *   nested group.
 */

const scanRepeatedBody = (body, options) => {
  let star = false;
  let unbounded = false;
  let mayMatchSlash = false;
  let depth = 0;
  let bracket = false;
  let quote = false;
  // The character before, as glob syntax: an escaped one never counts.
  let prev = '';

  for (let i = 0; i < body.length; i++) {
    const ch = body[i];
    const next = body[i + 1];
    const before = prev;
    prev = ch;

    if (ch === '\\') {
      // With `windows: true` an escaped backslash is a path separator, which
      // a `*` does not cross.
      if (next === '/' || (next === '\\' && options.windows === true)) mayMatchSlash = true;
      prev = '';
      i++;
      continue;
    }

    if (quote) {
      if (ch === '"') quote = false;
      continue;
    }

    if (bracket) {
      if (ch === ']') bracket = false;
      continue;
    }

    if (ch === '"') {
      quote = true;
      mayMatchSlash = true;
    } else if (ch === '[') {
      bracket = true;
      mayMatchSlash = true;
    } else if (ch === '/' || ch === '{' || ch === '}') {
      mayMatchSlash = true;
    } else if (ch === '(') {
      depth++;
      mayMatchSlash = true;
      if (before === '!') unbounded = true;
    } else if (ch === ')') {
      depth--;
    } else if (ch === '*') {
      if (next === '(') continue;
      unbounded = true;
      if (next === '*' || before === '*') {
        mayMatchSlash = true;
      } else if (depth === 0) {
        star = true;
      }
    } else if (ch === '+' && next !== '(') {
      unbounded = true;
    }
  }

  return { star, unbounded, mayMatchSlash };
};

/**
 * Hardwave fork (GHSA-fc5q-39xc-vjpf, CVE-2026-112366).
 *
 * A repeated extglob with one branch, `+(B)` or `*(B)`, puts whatever open
 * quantifier B holds under another one. Upstream only looked at bodies with
 * several branches, so `+(*)`, `+(*a)` or `+(@(*a))` compiled to regexes
 * such as `(?:[^/]*?a)+`, which backtrack exponentially on a short string
 * that almost matches.
 *
 * When B has a top-level `*` and cannot match a `/`, the repetition adds
 * nothing: B is P, then any text without a slash, then Q, so any run of
 * matches of B is P, then text without a slash, then Q, which is one match
 * of B. `+(B)` is then exactly `@(B)` and `*(B)` exactly `?(B)`, and that
 * is what it compiles to (`collapse`). Any other single branch is checked
 * with modelRepeatedBody, and one whose repetition can backtrack
 * exponentially is matched literally (`risky`), as upstream does with its
 * other risky repeated extglobs.
 */

const isSingleNegation = branch => {
  if (!branch.startsWith('!(') || branch.includes('/')) {
    return false;
  }

  let depth = 0;
  let bracket = false;
  let quote = false;

  for (let i = 1; i < branch.length; i++) {
    const ch = branch[i];

    if (ch === '\\') {
      i++;
    } else if (quote) {
      if (ch === '"') quote = false;
    } else if (bracket) {
      if (ch === ']') bracket = false;
    } else if (ch === '"') {
      quote = true;
    } else if (ch === '[') {
      bracket = true;
    } else if (ch === '(') {
      depth++;
    } else if (ch === ')') {
      depth--;
      if (depth === 0) return i === branch.length - 1;
    }
  }

  return false;
};

/**
 * Hardwave fork: a model of how a backtracking regex engine runs `+(B)`.
 *
 * B is read into a small expression tree: characters, `?`, `*`, `**`,
 * brackets, nested `@( )`, `?( )`, `*( )`, `+( )` and plain groups, and a
 * `+` quantifier. Brackets count as "any character, slash included", which
 * only makes the model see more overlap than the regex has, never less.
 * Anything else (`!( )` inside B, braces, quotes, POSIX classes) is not
 * modelled, and the caller falls back to the cruder reading of
 * scanRepeatedBody.
 *
 * The tree becomes a Thompson automaton, with the empty moves folded into
 * the character moves while counting how many distinct empty paths each one
 * stands for: that is where `(a+)+` hides its two ways round. A backtracking
 * matcher tries every path, so it takes exponential time on some failing
 * string exactly when the automaton has exponential ambiguity: two different
 * loops from one state back to it that read the same text. In the automaton
 * paired with itself that is a strongly connected part holding a state
 * (p, p) together with a state (p, q) with p != q, or with a doubled move
 * between two such states. A loop of empty moves (a repeated part that can
 * match nothing) counts as exponential too.
 */

const REPEAT_MODEL_MAX_CHARS = 24;

const modelRepeatedBody = (body, options) => {
  const literals = new Set();
  let unsupported = false;
  let chars = 0;

  const findClose = (str, open) => {
    let depth = 0;
    let bracket = false;

    for (let i = open; i < str.length; i++) {
      const ch = str[i];
      if (ch === '\\') {
        i++;
      } else if (bracket) {
        if (ch === ']') bracket = false;
      } else if (ch === '[') {
        bracket = true;
      } else if (ch === '(') {
        depth++;
      } else if (ch === ')') {
        depth--;
        if (depth === 0) return i;
      }
    }

    return -1;
  };

  const sym = (kind, value) => {
    chars++;
    if (kind === 'lit') literals.add(value);
    return { type: 'sym', kind, value };
  };

  const literal = ch => sym('lit', options.nocase === true ? ch.toLowerCase() : ch);

  // The characters a bracket stands for. A bracket starting with `!` or `^`
  // (which picomatch 4.0.7 passes on as `[!a]`, matching `!` or `a`), or
  // with a range too wide to list, counts as any character.
  const bracketClass = content => {
    if (content[0] === '!' || content[0] === '^') {
      chars++;
      return { type: 'sym', kind: 'any' };
    }
    const members = new Set();
    let wide = false;
    for (let k = 0; k < content.length; k++) {
      if (content[k + 1] === '-' && k + 2 < content.length) {
        const from = content.charCodeAt(k);
        const to = content.charCodeAt(k + 2);
        if (to - from > 256) wide = true;
        for (let c = from; c <= to && !wide; c++) members.add(String.fromCharCode(c));
        k += 2;
      } else {
        members.add(content[k]);
      }
    }
    const folded = [...members].map(ch => (options.nocase === true ? ch.toLowerCase() : ch));
    for (const ch of folded) literals.add(ch);
    chars++;
    return wide ? { type: 'sym', kind: 'any' } : { type: 'sym', kind: 'class', members: new Set(folded) };
  };

  const parseSeq = str => {
    const items = [];
    // Whether the last item read was a group: picomatch passes a `?` after
    // one through as a regex quantifier, and reads one at the start of a
    // group as a plain character.
    let afterGroup = false;

    for (let i = 0; i < str.length && !unsupported; i++) {
      const ch = str[i];
      const next = str[i + 1];
      const groupBefore = afterGroup;
      afterGroup = false;

      if (ch === '?' && i === 0) {
        items.push(literal(ch));
      } else if (ch === '?' && groupBefore && next !== '(') {
        items.push({ type: 'opt', node: items.pop() });
      } else if (ch === '\\') {
        if (next === undefined) {
          unsupported = true;
          break;
        }
        items.push(literal(next));
        i++;
      } else if ('*+?@!'.includes(ch) && next === '(') {
        const close = findClose(str, i + 1);
        if (ch === '!' || close < 0) {
          unsupported = true;
          break;
        }
        const inner = parseAlt(str.slice(i + 2, close));
        const type = { '*': 'star', '+': 'plus', '?': 'opt', '@': 'group' }[ch];
        items.push(type === 'group' ? inner : { type, node: inner });
        afterGroup = true;
        i = close;
      } else if (ch === '(') {
        const close = findClose(str, i);
        if (close < 0) {
          unsupported = true;
          break;
        }
        items.push(parseAlt(str.slice(i + 1, close)));
        afterGroup = true;
        i = close;
      } else if (ch === '*') {
        if (next === '*') {
          while (str[i + 1] === '*') i++;
          items.push({ type: 'star', node: sym('any') });
        } else {
          items.push({ type: 'star', node: sym('anyNoSlash') });
        }
      } else if (ch === '+') {
        items.push(items.length ? { type: 'plus', node: items.pop() } : literal(ch));
      } else if (ch === '?') {
        items.push(sym('anyNoSlash'));
      } else if (ch === '[') {
        const close = str.indexOf(']', i + 2);
        if (close < 0 || /\[:|\\/.test(str.slice(i + 1, close))) {
          unsupported = true;
          break;
        }
        // picomatch matches a bracket as its class or as its own text.
        const text = str.slice(i, close + 1).split('').map(literal);
        items.push({ type: 'alt', items: [bracketClass(str.slice(i + 1, close)), { type: 'seq', items: text }] });
        i = close;
      } else if ('{}"|)'.includes(ch)) {
        unsupported = true;
      } else {
        items.push(literal(ch));
      }
    }

    return { type: 'seq', items };
  };

  const parseAlt = str => {
    const branches = splitTopLevel(str).map(parseSeq);
    return branches.length === 1 ? branches[0] : { type: 'alt', items: branches };
  };

  const tree = parseAlt(body);
  if (unsupported || chars === 0 || chars > REPEAT_MODEL_MAX_CHARS) {
    return;
  }

  // With `windows: true` a `*` or `?` stops at a backslash as well.
  const separators = options.windows === true ? ['/', '\\'] : ['/'];

  const labelOf = node => {
    // With `windows: true` a `/` in the glob matches a backslash too.
    if (node.kind === 'lit' && separators.includes(node.value)) return new Set(separators);
    if (node.kind === 'lit') return new Set([node.value]);
    if (node.kind === 'class') {
      const set = new Set(node.members);
      if (separators.some(ch => set.has(ch))) separators.forEach(ch => set.add(ch));
      return set;
    }
    const set = new Set([...literals].filter(ch => node.kind === 'any' || !separators.includes(ch)));
    set.add('other');
    if (node.kind === 'any') set.add('/');
    return set;
  };

  // Thompson automaton: `empty[s]` lists empty moves, `move[s]` the one
  // character move out of s, if any.
  const empty = [];
  const move = [];
  const state = () => {
    empty.push([]);
    move.push(null);
    return empty.length - 1;
  };

  const build = node => {
    const start = state();
    const end = state();

    if (node.type === 'sym') {
      move[start] = { to: end, label: labelOf(node) };
    } else if (node.type === 'seq' || node.type === 'alt') {
      if (node.items.length === 0) empty[start].push(end);
      let prev = start;
      for (const item of node.items) {
        const part = build(item);
        if (node.type === 'seq') {
          empty[prev].push(part.start);
          prev = part.end;
        } else {
          empty[start].push(part.start);
          empty[part.end].push(end);
        }
      }
      if (node.type === 'seq' && node.items.length) empty[prev].push(end);
    } else {
      const part = build(node.node);
      empty[start].push(part.start);
      if (node.type !== 'opt') empty[part.end].push(part.start);
      if (node.type !== 'plus') empty[start].push(end);
      empty[part.end].push(end);
    }

    return { start, end };
  };

  const root = build({ type: 'plus', node: tree });
  const count = empty.length;

  // Order the empty moves; a loop among them is a repeated part that can
  // match nothing.
  const order = [];
  const mark = new Array(count).fill(0);
  const visit = s => {
    if (mark[s] === 1) return false;
    if (mark[s] === 2) return true;
    mark[s] = 1;
    for (const t of empty[s]) {
      if (!visit(t)) return false;
    }
    mark[s] = 2;
    order.push(s);
    return true;
  };
  for (let s = 0; s < count; s++) {
    if (!visit(s)) return { exponential: true };
  }
  order.reverse();

  // Fold the empty moves: from each state a character move can leave from
  // (the start, and every state a character move lands on), the character
  // moves reachable through empty moves, each with how many distinct empty
  // paths lead to it (counting stops at 2).
  const sources = [root.start, ...move.filter(Boolean).map(m => m.to)];
  const edges = new Map();
  const finals = new Set();

  for (const from of new Set(sources)) {
    const paths = new Array(count).fill(0);
    paths[from] = 1;
    for (const s of order) {
      if (paths[s] === 0) continue;
      for (const t of empty[s]) paths[t] = Math.min(2, paths[t] + paths[s]);
    }
    if (paths[root.end] > 0) finals.add(from);
    const out = [];
    for (let s = 0; s < count; s++) {
      if (paths[s] > 0 && move[s]) out.push({ to: move[s].to, label: move[s].label, times: paths[s] });
    }
    edges.set(from, out);
  }

  // The automaton paired with itself, kept to the pairs that lie on a run
  // from the start to an end.
  const key = (p, q) => `${p},${q}`;
  const overlap = (a, b) => [...a].some(ch => b.has(ch));
  const next = new Map();
  const prev = new Map();
  const doubled = new Set();
  const seen = new Set([key(root.start, root.start)]);
  const queue = [[root.start, root.start]];

  while (queue.length) {
    const [p, q] = queue.shift();
    const k = key(p, q);
    next.set(k, []);
    for (const a of edges.get(p)) {
      for (const b of edges.get(q)) {
        if (!overlap(a.label, b.label)) continue;
        const nk = key(a.to, b.to);
        next.get(k).push(nk);
        if (!prev.has(nk)) prev.set(nk, []);
        prev.get(nk).push(k);
        if (p === q && a === b && a.times > 1) doubled.add(`${k}>${nk}`);
        if (!seen.has(nk)) {
          seen.add(nk);
          queue.push([a.to, b.to]);
        }
      }
    }
  }

  const live = new Set();
  const back = [...seen].filter(k => k.split(',').every(s => finals.has(Number(s))));
  for (const k of back) live.add(k);
  while (back.length) {
    for (const pk of prev.get(back.pop()) || []) {
      if (!live.has(pk)) {
        live.add(pk);
        back.push(pk);
      }
    }
  }

  const reach = (from, links) => {
    const found = new Set([from]);
    const todo = [from];
    while (todo.length) {
      for (const k of links.get(todo.pop()) || []) {
        if (live.has(k) && !found.has(k)) {
          found.add(k);
          todo.push(k);
        }
      }
    }
    return found;
  };

  for (const k of live) {
    const [p, q] = k.split(',');
    if (p !== q) continue;
    const forward = reach(k, next);
    const backward = reach(k, prev);
    const part = [...forward].filter(x => backward.has(x));
    if (part.some(x => { const [a, b] = x.split(','); return a !== b; })) {
      return { exponential: true };
    }
    for (const x of part) {
      for (const y of part) {
        if (doubled.has(`${x}>${y}`)) return { exponential: true };
      }
    }
  }

  return { exponential: false };
};

const analyzeSingleBranch = (branch, options, max) => {
  const { star, unbounded, mayMatchSlash } = scanRepeatedBody(branch, options);

  if (star && !mayMatchSlash) {
    return { risky: false, collapse: true };
  }

  // `!(X)` compiles to a look-ahead on X followed by a `*` that takes any
  // text without a slash, so the same holds for `+(!(X))` and `*(!(X))`:
  // the first repetition's look-ahead is the only one that decides, and one
  // repetition takes everything the others would.
  if (isSingleNegation(branch)) {
    return { risky: false, collapse: true };
  }

  // A caller that raised maxExtglobRecursion has accepted repeated extglobs
  // nested to that depth, and gets them as upstream compiles them.
  if (max > 0 && /[*+]\(/.test(branch)) {
    return { risky: false };
  }

  // Anything else is matched literally when repeating it can backtrack
  // exponentially (`+(*(a)a)`, `+(a+)`, `+(*[a-z])`), and left as it is
  // when it cannot (`*(*(of*(o)x)o)`, `+(a+b)`). A body the model does not
  // read is matched literally whenever it holds an open-ended quantifier.
  const model = modelRepeatedBody(branch, options);
  if (model) {
    return { risky: model.exponential };
  }

  return { risky: unbounded };
};

/**
 * Hardwave fork: several branches, each starting with a plain `*` (or each
 * ending with one), none able to match a slash: `+(*.js|*.ts)`,
 * `+(*a|*a)`. Whatever a run of matches holds before the last one is text
 * without a slash, which that last branch's leading `*` takes, so the run is
 * one match; the same holds the other way round for trailing stars. The
 * repetition is dropped as for a single branch. Other sets of branches
 * keep upstream's handling (see vendor/picomatch/README.md for the ones
 * that still backtrack).
 */

const plainStarAt = (branch, index) => {
  if (branch[index] !== '*' || branch[index - 1] === '*' || branch[index + 1] === '*') {
    return false;
  }
  if (branch[index + 1] === '(') return false;
  let slashes = 0;
  for (let k = index - 1; k >= 0 && branch[k] === '\\'; k--) slashes++;
  return slashes % 2 === 0;
};

const analyzeBranches = (branches, options) => {
  if (branches.some(branch => scanRepeatedBody(branch, options).mayMatchSlash)) {
    return { risky: false };
  }

  const leading = branches.every(branch => plainStarAt(branch, 0));
  const trailing = branches.every(branch => plainStarAt(branch, branch.length - 1));
  return leading || trailing ? { risky: false, collapse: true } : { risky: false };
};

const analyzeRepeatedExtglob = (body, options) => {
  if (options.maxExtglobRecursion === false) {
    return { risky: false };
  }

  const max =
    typeof options.maxExtglobRecursion === 'number'
      ? options.maxExtglobRecursion
      : constants.DEFAULT_MAX_EXTGLOB_RECURSION;

  const branches = splitTopLevel(body).map(branch => branch.trim());

  if (branches.length > 1) {
    if (
      branches.some(branch => branch === '') ||
      branches.some(branch => /^[*?]+$/.test(branch)) ||
      hasRepeatedCharPrefixOverlap(branches)
    ) {
      return { risky: true };
    }
  }

  // A repeated extglob is "risky" (prone to catastrophic backtracking) when a
  // branch is itself a `*(...)` sequence, since that nests an unbounded quantifier
  // inside the outer `+(...)`/`*(...)`. When *every* branch reduces to single
  // characters we can emit one flat, ReDoS-safe character class that preserves the
  // meaning of ALL branches (e.g. `+(*(a)|*(b))` -> `[ab]*`), rather than dropping
  // every branch but the first.
  const safeChars = [];
  let sawStarSequence = false;
  let combinable = true;

  for (const branch of branches) {
    const chars = getStarExtglobSequenceChars(branch);
    if (chars) {
      sawStarSequence = true;
      safeChars.push(...chars);
      continue;
    }

    const literal = normalizeSimpleBranch(branch);
    if (literal && literal.length === 1) {
      safeChars.push(literal);
      continue;
    }

    combinable = false;

    if (repeatedExtglobRecursion(branch) > max) {
      return { risky: true };
    }
  }

  if (sawStarSequence) {
    return combinable
      ? { risky: true, safeOutput: buildCharClassStar([...new Set(safeChars)]) }
      : { risky: true };
  }

  if (branches.length === 1) {
    return analyzeSingleBranch(branches[0], options, max);
  }

  return analyzeBranches(branches, options);
};

/**
 * Parse the given input string.
 * @param {String} input
 * @param {Object} options
 * @return {Object}
 */

const parse = (input, options) => {
  if (typeof input !== 'string') {
    throw new TypeError('Expected a string');
  }

  input = REPLACEMENTS[input] || input;

  const opts = { ...options };
  const max = typeof opts.maxLength === 'number' ? Math.min(MAX_LENGTH, opts.maxLength) : MAX_LENGTH;

  let len = input.length;
  if (len > max) {
    throw new SyntaxError(`Input length: ${len}, exceeds maximum allowed length: ${max}`);
  }

  const bos = { type: 'bos', value: '', output: opts.prepend || '' };
  const tokens = [bos];

  const capture = opts.capture ? '' : '?:';

  // create constants based on platform, for windows or posix
  const PLATFORM_CHARS = constants.globChars(opts.windows);
  const EXTGLOB_CHARS = constants.extglobChars(PLATFORM_CHARS);

  const {
    DOT_LITERAL,
    PLUS_LITERAL,
    SLASH_LITERAL,
    ONE_CHAR,
    DOTS_SLASH,
    NO_DOT,
    NO_DOT_SLASH,
    NO_DOTS_SLASH,
    QMARK,
    QMARK_NO_DOT,
    STAR,
    START_ANCHOR
  } = PLATFORM_CHARS;

  const globstar = opts => {
    return `(${capture}(?:(?!${START_ANCHOR}${opts.dot ? DOTS_SLASH : DOT_LITERAL}).)*?)`;
  };

  const nodot = opts.dot ? '' : NO_DOT;
  const qmarkNoDot = opts.dot ? QMARK : QMARK_NO_DOT;
  let star = opts.bash === true ? globstar(opts) : STAR;

  if (opts.capture) {
    star = `(${star})`;
  }

  // minimatch options support
  if (typeof opts.noext === 'boolean') {
    opts.noextglob = opts.noext;
  }

  const state = {
    input,
    index: -1,
    start: 0,
    dot: opts.dot === true,
    consumed: '',
    output: '',
    prefix: '',
    backtrack: false,
    negated: false,
    brackets: 0,
    braces: 0,
    parens: 0,
    quotes: 0,
    globstar: false,
    tokens
  };

  input = utils.removePrefix(input, state);
  len = input.length;

  const extglobs = [];
  const braces = [];
  const stack = [];
  let prev = bos;
  let value;

  /**
   * Tokenizing helpers
   */

  const eos = () => state.index === len - 1;
  const peek = state.peek = (n = 1) => input[state.index + n];
  const advance = state.advance = () => input[++state.index] || '';
  const remaining = () => input.slice(state.index + 1);
  const consume = (value = '', num = 0) => {
    state.consumed += value;
    state.index += num;
  };

  const append = token => {
    state.output += token.output != null ? token.output : token.value;
    consume(token.value);
  };

  const negate = () => {
    let count = 1;

    while (peek() === '!' && (peek(2) !== '(' || peek(3) === '?')) {
      advance();
      state.start++;
      count++;
    }

    if (count % 2 === 0) {
      return false;
    }

    state.negated = true;
    state.start++;
    return true;
  };

  const increment = type => {
    state[type]++;
    stack.push(type);
  };

  const decrement = type => {
    state[type]--;
    stack.pop();
  };

  /**
   * Push tokens onto the tokens array. This helper speeds up
   * tokenizing by 1) helping us avoid backtracking as much as possible,
   * and 2) helping us avoid creating extra tokens when consecutive
   * characters are plain text. This improves performance and simplifies
   * lookbehinds.
   */

  const push = tok => {
    if (prev.type === 'globstar') {
      const isBrace = state.braces > 0 && (tok.type === 'comma' || tok.type === 'brace');
      const isExtglob = tok.extglob === true || (extglobs.length && (tok.type === 'pipe' || tok.type === 'paren'));

      if (tok.type !== 'slash' && tok.type !== 'paren' && !isBrace && !isExtglob) {
        state.output = state.output.slice(0, -prev.output.length);
        prev.type = 'star';
        prev.value = '*';
        prev.output = star;
        state.output += prev.output;
      }
    }

    if (extglobs.length && tok.type !== 'paren') {
      extglobs[extglobs.length - 1].inner += tok.value;
    }

    if (tok.value || tok.output) append(tok);
    if (prev && prev.type === 'text' && tok.type === 'text') {
      prev.output = (prev.output || prev.value) + tok.value;
      prev.value += tok.value;
      return;
    }

    tok.prev = prev;
    tokens.push(tok);
    prev = tok;
  };

  const extglobOpen = (type, value) => {
    const token = { ...EXTGLOB_CHARS[value], conditions: 1, inner: '' };

    token.prev = prev;
    token.parens = state.parens;
    token.output = state.output;
    token.startIndex = state.index;
    token.tokensIndex = tokens.length;
    const output = (opts.capture ? '(' : '') + token.open;

    increment('parens');
    push({ type, value, output: state.output ? '' : ONE_CHAR });
    push({ type: 'paren', extglob: true, value: advance(), output });
    extglobs.push(token);
  };

  const extglobClose = token => {
    const literal = input.slice(token.startIndex, state.index + 1);
    const body = input.slice(token.startIndex + 2, state.index);
    const analysis = analyzeRepeatedExtglob(body, opts);

    if ((token.type === 'plus' || token.type === 'star') && analysis.risky) {
      const safeOutput = analysis.safeOutput
        ? (token.output ? '' : ONE_CHAR) + (opts.capture ? `(${analysis.safeOutput})` : analysis.safeOutput)
        : undefined;
      const open = tokens[token.tokensIndex];

      open.type = 'text';
      open.value = literal;
      open.output = safeOutput || utils.escapeRegex(literal);

      for (let i = token.tokensIndex + 1; i < tokens.length; i++) {
        tokens[i].value = '';
        tokens[i].output = '';
        delete tokens[i].suffix;
      }

      state.output = token.output + open.output;
      state.backtrack = true;

      push({ type: 'paren', extglob: true, value, output: '' });
      decrement('parens');
      return;
    }

    // Hardwave fork: `+(B)` matches as `@(B)` and `*(B)` as `?(B)` when
    // repeating B cannot match anything B alone does not (see
    // analyzeSingleBranch), so no quantifier is nested under another.
    let close = token.close;
    if (analysis.collapse === true && (token.type === 'plus' || token.type === 'star')) {
      close = token.type === 'plus' ? ')' : ')?';
    }

    let output = close + (opts.capture ? ')' : '');
    let rest;

    if (token.type === 'negate') {
      let extglobStar = star;

      if (token.inner && token.inner.length > 1 && token.inner.includes('/')) {
        extglobStar = globstar(opts);
      }

      if (extglobStar !== star || eos() || /^\)+$/.test(remaining())) {
        output = token.close = `)$))${extglobStar}`;
      }

      if (token.inner.includes('*') && (rest = remaining()) && /^\.[^\\/.]+$/.test(rest)) {
        // Any non-magical string (`.ts`) or even nested expression (`.{ts,tsx}`) can follow after the closing parenthesis.
        // In this case, we need to parse the string and use it in the output of the original pattern.
        // Suitable patterns: `/!(*.d).ts`, `/!(*.d).{ts,tsx}`, `**/!(*-dbg).@(js)`.
        //
        // Disabling the `fastpaths` option due to a problem with parsing strings as `.ts` in the pattern like `**/!(*.d).ts`.
        const expression = parse(rest, { ...options, fastpaths: false }).output;

        output = token.close = `)${expression})${extglobStar})`;
      }

      if (token.prev.type === 'bos') {
        state.negatedExtglob = true;
      }
    }

    push({ type: 'paren', extglob: true, value, output });
    decrement('parens');
  };

  /**
   * Fast paths
   */

  if (opts.fastpaths !== false && !/(^[*!]|[/()[\]{}"])/.test(input)) {
    let backslashes = false;

    let output = input.replace(REGEX_SPECIAL_CHARS_BACKREF, (m, esc, chars, first, rest, index) => {
      if (first === '\\') {
        backslashes = true;
        return m;
      }

      if (first === '?') {
        if (esc) {
          return esc + first + (rest ? QMARK.repeat(rest.length) : '');
        }
        if (index === 0) {
          return qmarkNoDot + (rest ? QMARK.repeat(rest.length) : '');
        }
        return QMARK.repeat(chars.length);
      }

      if (first === '.') {
        return DOT_LITERAL.repeat(chars.length);
      }

      if (first === '*') {
        if (esc) {
          return esc + first + (rest ? star : '');
        }
        return star;
      }
      return esc ? m : `\\${m}`;
    });

    if (backslashes === true) {
      if (opts.unescape === true) {
        output = output.replace(/\\/g, '');
      } else {
        output = output.replace(/\\+/g, m => {
          return m.length % 2 === 0 ? '\\\\' : (m ? '\\' : '');
        });
      }
    }

    if (output === input && opts.contains === true) {
      state.output = input;
      return state;
    }

    state.output = utils.wrapOutput(output, state, options);
    return state;
  }

  /**
   * Tokenize input until we reach end-of-string
   */

  while (!eos()) {
    value = advance();

    if (value === '\u0000') {
      continue;
    }

    /**
     * Escaped characters
     */

    if (value === '\\') {
      const next = peek();

      if (next === '/' && opts.bash !== true) {
        continue;
      }

      if (next === '.' || next === ';') {
        continue;
      }

      if (!next) {
        value += '\\';
        push({ type: 'text', value });
        continue;
      }

      // collapse slashes to reduce potential for exploits
      const match = /^\\+/.exec(remaining());
      let slashes = 0;

      if (match && match[0].length > 2) {
        slashes = match[0].length;
        state.index += slashes;
        if (slashes % 2 !== 0) {
          value += '\\';
        }
      }

      if (opts.unescape === true) {
        value = advance();
      } else {
        value += advance();
      }

      if (state.brackets === 0) {
        push({ type: 'text', value });
        continue;
      }
    }

    /**
     * If we're inside a regex character class, continue
     * until we reach the closing bracket.
     */

    if (state.brackets > 0 && (value !== ']' || prev.value === '[' || prev.value === '[^')) {
      if (opts.posix !== false && value === ':') {
        const inner = prev.value.slice(1);
        if (inner.includes('[')) {
          prev.posix = true;

          if (inner.includes(':')) {
            const idx = prev.value.lastIndexOf('[');
            const pre = prev.value.slice(0, idx);
            const rest = prev.value.slice(idx + 2);
            const posix = POSIX_REGEX_SOURCE[rest];
            if (posix) {
              prev.value = pre + posix;
              state.backtrack = true;
              advance();

              if (!bos.output && tokens.indexOf(prev) === 1) {
                bos.output = ONE_CHAR;
              }
              continue;
            }
          }
        }
      }

      if ((value === '[' && peek() !== ':') || (value === '-' && peek() === ']')) {
        value = `\\${value}`;
      }

      if (value === ']' && (prev.value === '[' || prev.value === '[^')) {
        value = `\\${value}`;
      }

      if (opts.posix === true && value === '!' && prev.value === '[') {
        value = '^';
      }

      prev.value += value;
      append({ value });
      continue;
    }

    /**
     * If we're inside a quoted string, continue
     * until we reach the closing double quote.
     */

    if (state.quotes === 1 && value !== '"') {
      value = utils.escapeRegex(value);
      prev.value += value;
      append({ value });
      continue;
    }

    /**
     * Double quotes
     */

    if (value === '"') {
      state.quotes = state.quotes === 1 ? 0 : 1;
      if (opts.keepQuotes === true) {
        push({ type: 'text', value });
      }
      continue;
    }

    /**
     * Parentheses
     */

    if (value === '(') {
      increment('parens');
      push({ type: 'paren', value });
      continue;
    }

    if (value === ')') {
      if (state.parens === 0 && opts.strictBrackets === true) {
        throw new SyntaxError(syntaxError('opening', '('));
      }

      const extglob = extglobs[extglobs.length - 1];
      if (extglob && state.parens === extglob.parens + 1) {
        extglobClose(extglobs.pop());
        continue;
      }

      push({ type: 'paren', value, output: state.parens ? ')' : '\\)' });
      decrement('parens');
      continue;
    }

    /**
     * Square brackets
     */

    if (value === '[') {
      if (opts.nobracket === true || !remaining().includes(']')) {
        if (opts.nobracket !== true && opts.strictBrackets === true) {
          throw new SyntaxError(syntaxError('closing', ']'));
        }

        value = `\\${value}`;
      } else {
        increment('brackets');
      }

      push({ type: 'bracket', value });
      continue;
    }

    if (value === ']') {
      if (opts.nobracket === true || (prev && prev.type === 'bracket' && prev.value.length === 1)) {
        push({ type: 'text', value, output: `\\${value}` });
        continue;
      }

      if (state.brackets === 0) {
        if (opts.strictBrackets === true) {
          throw new SyntaxError(syntaxError('opening', '['));
        }

        push({ type: 'text', value, output: `\\${value}` });
        continue;
      }

      decrement('brackets');

      const prevValue = prev.value.slice(1);
      if (prev.posix !== true && prevValue[0] === '^' && !prevValue.includes('/')) {
        value = `/${value}`;
      }

      prev.value += value;
      append({ value });

      // when literal brackets are explicitly disabled
      // assume we should match with a regex character class
      if (opts.literalBrackets === false || utils.hasRegexChars(prevValue)) {
        continue;
      }

      const escaped = utils.escapeRegex(prev.value);
      state.output = state.output.slice(0, -prev.value.length);

      // when literal brackets are explicitly enabled
      // assume we should escape the brackets to match literal characters
      if (opts.literalBrackets === true) {
        state.output += escaped;
        prev.value = escaped;
        continue;
      }

      // when the user specifies nothing, try to match both
      prev.value = `(${capture}${escaped}|${prev.value})`;
      state.output += prev.value;
      continue;
    }

    /**
     * Braces
     */

    if (value === '{' && opts.nobrace !== true) {
      increment('braces');

      const open = {
        type: 'brace',
        value,
        output: '(',
        outputIndex: state.output.length,
        tokensIndex: state.tokens.length
      };

      braces.push(open);
      push(open);
      continue;
    }

    if (value === '}') {
      const brace = braces[braces.length - 1];

      if (opts.nobrace === true || !brace) {
        push({ type: 'text', value, output: value });
        continue;
      }

      let output = ')';

      if (brace.dots === true) {
        const arr = tokens.slice();
        const range = [];

        for (let i = arr.length - 1; i >= 0; i--) {
          tokens.pop();
          if (arr[i].type === 'brace') {
            break;
          }
          if (arr[i].type !== 'dots') {
            range.unshift(arr[i].value);
          }
        }

        output = expandRange(range, opts);
        state.backtrack = true;
      }

      if (brace.comma !== true && brace.dots !== true) {
        const out = state.output.slice(0, brace.outputIndex);
        const toks = state.tokens.slice(brace.tokensIndex);
        brace.value = brace.output = '\\{';
        value = output = '\\}';
        state.output = out;
        for (const t of toks) {
          state.output += (t.output || t.value);
        }
      }

      push({ type: 'brace', value, output });
      decrement('braces');
      braces.pop();
      continue;
    }

    /**
     * Pipes
     */

    if (value === '|') {
      if (extglobs.length > 0) {
        extglobs[extglobs.length - 1].conditions++;
      }
      push({ type: 'text', value });
      continue;
    }

    /**
     * Commas
     */

    if (value === ',') {
      let output = value;

      const brace = braces[braces.length - 1];
      if (brace && stack[stack.length - 1] === 'braces') {
        brace.comma = true;
        output = '|';
      }

      push({ type: 'comma', value, output });
      continue;
    }

    /**
     * Slashes
     */

    if (value === '/') {
      // if the beginning of the glob is "./", advance the start
      // to the current index, and don't add the "./" characters
      // to the state. This greatly simplifies lookbehinds when
      // checking for BOS characters like "!" and "." (not "./")
      if (prev.type === 'dot' && state.index === state.start + 1) {
        state.start = state.index + 1;
        state.consumed = '';
        state.output = '';
        tokens.pop();
        prev = bos; // reset "prev" to the first token
        continue;
      }

      push({ type: 'slash', value, output: SLASH_LITERAL });
      continue;
    }

    /**
     * Dots
     */

    if (value === '.') {
      if (state.braces > 0 && prev.type === 'dot') {
        if (prev.value === '.') prev.output = DOT_LITERAL;
        const brace = braces[braces.length - 1];
        prev.type = 'dots';
        prev.output += value;
        prev.value += value;
        brace.dots = true;
        continue;
      }

      if ((state.braces + state.parens) === 0 && prev.type !== 'bos' && prev.type !== 'slash') {
        push({ type: 'text', value, output: DOT_LITERAL });
        continue;
      }

      push({ type: 'dot', value, output: DOT_LITERAL });
      continue;
    }

    /**
     * Question marks
     */

    if (value === '?') {
      const isGroup = prev && prev.value === '(';
      if (!isGroup && opts.noextglob !== true && peek() === '(' && peek(2) !== '?') {
        extglobOpen('qmark', value);
        continue;
      }

      if (prev && prev.type === 'paren') {
        const next = peek();
        let output = value;

        if ((prev.value === '(' && !/[!=<:]/.test(next)) || (next === '<' && !/<([!=]|\w+>)/.test(remaining()))) {
          output = `\\${value}`;
        }

        push({ type: 'text', value, output });
        continue;
      }

      if (opts.dot !== true && (prev.type === 'slash' || prev.type === 'bos')) {
        push({ type: 'qmark', value, output: QMARK_NO_DOT });
        continue;
      }

      push({ type: 'qmark', value, output: QMARK });
      continue;
    }

    /**
     * Exclamation
     */

    if (value === '!') {
      if (opts.noextglob !== true && peek() === '(') {
        if (peek(2) !== '?' || !/[!=<:]/.test(peek(3))) {
          extglobOpen('negate', value);
          continue;
        }
      }

      if (opts.nonegate !== true && state.index === 0) {
        negate();
        continue;
      }
    }

    /**
     * Plus
     */

    if (value === '+') {
      if (opts.noextglob !== true && peek() === '(' && peek(2) !== '?') {
        extglobOpen('plus', value);
        continue;
      }

      if ((prev && prev.value === '(') || opts.regex === false) {
        push({ type: 'plus', value, output: PLUS_LITERAL });
        continue;
      }

      if ((prev && (prev.type === 'bracket' || prev.type === 'paren' || prev.type === 'brace')) || state.parens > 0) {
        push({ type: 'plus', value });
        continue;
      }

      push({ type: 'plus', value: PLUS_LITERAL });
      continue;
    }

    /**
     * Plain text
     */

    if (value === '@') {
      if (opts.noextglob !== true && peek() === '(' && peek(2) !== '?') {
        push({ type: 'at', extglob: true, value, output: '' });
        continue;
      }

      push({ type: 'text', value });
      continue;
    }

    /**
     * Plain text
     */

    if (value !== '*') {
      if (value === '$' || value === '^') {
        value = `\\${value}`;
      }

      const match = REGEX_NON_SPECIAL_CHARS.exec(remaining());
      if (match) {
        value += match[0];
        state.index += match[0].length;
      }

      push({ type: 'text', value });
      continue;
    }

    /**
     * Stars
     */

    if (prev && (prev.type === 'globstar' || prev.star === true)) {
      prev.type = 'star';
      prev.star = true;
      prev.value += value;
      prev.output = star;
      state.backtrack = true;
      state.globstar = true;
      consume(value);
      continue;
    }

    let rest = remaining();
    if (opts.noextglob !== true && /^\([^?]/.test(rest)) {
      extglobOpen('star', value);
      continue;
    }

    if (prev.type === 'star') {
      if (opts.noglobstar === true) {
        consume(value);
        continue;
      }

      const prior = prev.prev;
      const before = prior.prev;
      const isStart = prior.type === 'slash' || prior.type === 'bos';
      const afterStar = before && (before.type === 'star' || before.type === 'globstar');

      if (opts.bash === true && (!isStart || (rest[0] && rest[0] !== '/'))) {
        push({ type: 'star', value, output: '' });
        continue;
      }

      const isBrace = state.braces > 0 && (prior.type === 'comma' || prior.type === 'brace');
      const isExtglob = extglobs.length && (prior.type === 'pipe' || prior.type === 'paren');
      if (!isStart && prior.type !== 'paren' && !isBrace && !isExtglob) {
        push({ type: 'star', value, output: '' });
        continue;
      }

      // strip consecutive `/**/`
      while (rest.slice(0, 3) === '/**') {
        const after = input[state.index + 4];
        if (after && after !== '/') {
          break;
        }
        rest = rest.slice(3);
        consume('/**', 3);
      }

      // A globstar followed only by balanced closing parens is at the logical end of patterns like
      // `test(/utils/**)` and `test?(/utils/**)`. Treat it as EOS so the trailing `/**` can match its
      // parent path, except in negated extglobs where that would change the exclusion semantics.
      const isEnd = eos() || (
        state.parens > 0
        && rest === ')'.repeat(state.parens)
        && !extglobs.some(extglob => extglob.type === 'negate')
      );

      if (prior.type === 'bos' && eos()) {
        prev.type = 'globstar';
        prev.value += value;
        prev.output = globstar(opts);
        state.output = prev.output;
        state.globstar = true;
        consume(value);
        continue;
      }

      if (prior.type === 'slash' && prior.prev.type !== 'bos' && !afterStar && isEnd) {
        state.output = state.output.slice(0, -(prior.output + prev.output).length);
        prior.output = `(?:${prior.output}`;

        prev.type = 'globstar';
        prev.output = globstar(opts) + (opts.strictSlashes ? ')' : '|$)');
        prev.value += value;
        state.globstar = true;
        state.output += prior.output + prev.output;
        consume(value);
        continue;
      }

      if (prior.type === 'slash' && prior.prev.type !== 'bos' && rest[0] === '/') {
        const end = rest[1] !== void 0 ? '|$' : '';

        state.output = state.output.slice(0, -(prior.output + prev.output).length);
        prior.output = `(?:${prior.output}`;

        prev.type = 'globstar';
        prev.output = `${globstar(opts)}${SLASH_LITERAL}|${SLASH_LITERAL}${end})`;
        prev.value += value;

        state.output += prior.output + prev.output;
        state.globstar = true;

        consume(value + advance());

        push({ type: 'slash', value: '/', output: '' });
        continue;
      }

      if (prior.type === 'bos' && rest[0] === '/') {
        prev.type = 'globstar';
        prev.value += value;
        prev.output = `(?:^|${SLASH_LITERAL}|${globstar(opts)}${SLASH_LITERAL})`;
        state.output = prev.output;
        state.globstar = true;
        consume(value + advance());
        push({ type: 'slash', value: '/', output: '' });
        continue;
      }

      // remove single star from output
      state.output = state.output.slice(0, -prev.output.length);

      // reset previous token to globstar
      prev.type = 'globstar';
      prev.output = globstar(opts);
      prev.value += value;

      // reset output with globstar
      state.output += prev.output;
      state.globstar = true;
      consume(value);
      continue;
    }

    const token = { type: 'star', value, output: star };

    if (opts.bash === true) {
      token.output = '.*?';
      if (prev.type === 'bos' || prev.type === 'slash') {
        token.output = nodot + token.output;
      }
      push(token);
      continue;
    }

    if (prev && (prev.type === 'bracket' || prev.type === 'paren') && opts.regex === true) {
      token.output = value;
      push(token);
      continue;
    }

    if (state.index === state.start || prev.type === 'slash' || prev.type === 'dot') {
      if (prev.type === 'dot') {
        state.output += NO_DOT_SLASH;
        prev.output += NO_DOT_SLASH;

      } else if (opts.dot === true) {
        state.output += NO_DOTS_SLASH;
        prev.output += NO_DOTS_SLASH;

      } else {
        state.output += nodot;
        prev.output += nodot;
      }

      if (peek() !== '*') {
        state.output += ONE_CHAR;
        prev.output += ONE_CHAR;
      }
    }

    push(token);
  }

  while (state.brackets > 0) {
    if (opts.strictBrackets === true) throw new SyntaxError(syntaxError('closing', ']'));
    state.output = utils.escapeLast(state.output, '[');
    decrement('brackets');
  }

  while (state.parens > 0) {
    if (opts.strictBrackets === true) throw new SyntaxError(syntaxError('closing', ')'));
    state.output = utils.escapeLast(state.output, '(');
    decrement('parens');
  }

  while (state.braces > 0) {
    if (opts.strictBrackets === true) throw new SyntaxError(syntaxError('closing', '}'));
    state.output = utils.escapeLast(state.output, '{');
    decrement('braces');
  }

  if (opts.strictSlashes !== true && (prev.type === 'star' || prev.type === 'bracket')) {
    push({ type: 'maybe_slash', value: '', output: `${SLASH_LITERAL}?` });
  }

  // rebuild the output if we had to backtrack at any point
  if (state.backtrack === true) {
    state.output = '';

    for (const token of state.tokens) {
      state.output += token.output != null ? token.output : token.value;

      if (token.suffix) {
        state.output += token.suffix;
      }
    }
  }

  return state;
};

/**
 * Fast paths for creating regular expressions for common glob patterns.
 * This can significantly speed up processing and has very little downside
 * impact when none of the fast paths match.
 */

parse.fastpaths = (input, options) => {
  const opts = { ...options };
  const max = typeof opts.maxLength === 'number' ? Math.min(MAX_LENGTH, opts.maxLength) : MAX_LENGTH;
  const len = input.length;
  if (len > max) {
    throw new SyntaxError(`Input length: ${len}, exceeds maximum allowed length: ${max}`);
  }

  input = REPLACEMENTS[input] || input;

  // create constants based on platform, for windows or posix
  const {
    DOT_LITERAL,
    SLASH_LITERAL,
    ONE_CHAR,
    DOTS_SLASH,
    NO_DOT,
    NO_DOTS,
    NO_DOTS_SLASH,
    STAR,
    START_ANCHOR
  } = constants.globChars(opts.windows);

  const nodot = opts.dot ? NO_DOTS : NO_DOT;
  const slashDot = opts.dot ? NO_DOTS_SLASH : NO_DOT;
  const capture = opts.capture ? '' : '?:';
  const state = { negated: false, prefix: '' };
  let star = opts.bash === true ? '.*?' : STAR;

  if (opts.capture) {
    star = `(${star})`;
  }

  const globstar = opts => {
    if (opts.noglobstar === true) return star;
    return `(${capture}(?:(?!${START_ANCHOR}${opts.dot ? DOTS_SLASH : DOT_LITERAL}).)*?)`;
  };

  const create = str => {
    switch (str) {
      case '*':
        return `${nodot}${ONE_CHAR}${star}`;

      case '.*':
        return `${DOT_LITERAL}${ONE_CHAR}${star}`;

      case '*.*':
        return `${nodot}${star}${DOT_LITERAL}${ONE_CHAR}${star}`;

      case '*/*':
        return `${nodot}${star}${SLASH_LITERAL}${ONE_CHAR}${slashDot}${star}`;

      case '**':
        return nodot + globstar(opts);

      case '**/*':
        return `(?:${nodot}${globstar(opts)}${SLASH_LITERAL})?${slashDot}${ONE_CHAR}${star}`;

      case '**/*.*':
        return `(?:${nodot}${globstar(opts)}${SLASH_LITERAL})?${slashDot}${star}${DOT_LITERAL}${ONE_CHAR}${star}`;

      case '**/.*':
        return `(?:${nodot}${globstar(opts)}${SLASH_LITERAL})?${DOT_LITERAL}${ONE_CHAR}${star}`;

      default: {
        const match = /^(.*?)\.(\w+)$/.exec(str);
        if (!match) return;

        const source = create(match[1]);
        if (!source) return;

        return source + DOT_LITERAL + match[2];
      }
    }
  };

  const output = utils.removePrefix(input, state);
  let source = create(output);

  if (source && opts.strictSlashes !== true) {
    source += `${SLASH_LITERAL}?`;
  }

  return source;
};

module.exports = parse;
