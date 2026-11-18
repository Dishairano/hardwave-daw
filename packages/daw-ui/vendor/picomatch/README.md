# @hardwave/picomatch

picomatch 4.0.7 (MIT, by Jon Schlinkert) with a fix for GHSA-fc5q-39xc-vjpf
(CVE-2026-112366): a repeated extglob such as `+(*a)` compiled to a regex that
backtracks exponentially on a short string that almost matches. Every
picomatch release up to 4.0.7 is affected and there is no fixed release.

packages/daw-ui loads this copy instead of the registry package, through a
`file:` devDependency and `"overrides": { "picomatch": "$picomatch" }`; the
installer does the same with a path back to this folder. Only build and test
tools use it (vite, vitest, tinyglobby, fdir, and in the installer tailwind's
fast-glob, micromatch, anymatch and readdirp). Nothing of it ships in the app.

## What differs from 4.0.7

Only `lib/parse.js`, in the parts marked "Hardwave fork", plus this README,
`index.d.ts`, `package.json`, `test/` and `fuzz/`. The other files are byte
for byte the published 4.0.7.

For a repeated extglob `+(...)` or `*(...)`:

1. One branch with a top-level `*` that cannot match a slash (`+(*a)`,
   `+(a*b)`): repeating it matches nothing one match does not, so `+(B)`
   compiles as `@(B)` and `*(B)` as `?(B)`. Same matches, no nested
   quantifier. The same holds for one negation, `+(!(x))`, and for several
   branches that all start, or all end, with such a `*` (`+(*.js|*.ts)`).
   When a `+` or `?` follows the extglob, picomatch passes it on as a regex
   quantifier, which the rewrite would change, so the extglob is matched
   literally instead.
2. Any other branch, or set of branches holding more than plain `*`s, is
   read into a small automaton that counts the ways a backtracking engine
   can match the same text. If repeating it can take exponential time
   (`+(*(a)a)`, `+(a+)`, `+(a*(a)|b)`) the extglob is matched literally, as
   upstream already does with its other risky repeated extglobs. If not
   (`*(*(of*(o)x)o)`, `+(a+b)`, `+(*/)`), it compiles as upstream compiles
   it.
3. Every change is checked against the whole pattern. Wherever the fork
   changed something, `parse` also compiles the pattern exactly as 4.0.7
   does (the `hardwaveFork: false` option), and uses 4.0.7's result when
   either regex is invalid, when the fork's reading fails, or when the
   whole pattern, as the fork compiled it, cannot be shown free of
   exponential backtracking. So the fork never makes a pattern slower than
   4.0.7 and never matches differently, except by matching a risky repeated
   extglob literally.
4. `maxExtglobRecursion: false` turns all of this off, and a raised
   `maxExtglobRecursion` keeps nested repeated extglobs as upstream compiles
   them, as upstream documents.

The automaton reads glob text up to 256 characters, 16 levels of nesting
and 96 character positions, and gives up past a fixed amount of work; past
those limits a body counts as unread (rule 2 then matches it literally only
when it holds an open-ended quantifier) and a whole pattern as unproven (rule
3 then keeps 4.0.7's result).

## Tests

- `node --test test/`: the advisory's patterns must answer within 100 ms
  (each takes seconds on 4.0.7: run `PICOMATCH=<path to 4.0.7> node --test
  test/` to see it); every rewritten or kept pattern must match exactly
  what 4.0.7 matches, on all strings up to five characters; and 300 random
  globs go through the differential fuzz below.
- `node fuzz/differential.js [count] [seed] [length]`: random globs compiled
  by the fork and by 4.0.7. Where they differ, the fork must match the same
  strings (unless it fell back to a literal match) and may take at most 3x
  4.0.7's time plus 20 ms on hostile strings. 100,000 globs (five seeds,
  400,000 compilations) gave no mismatch and no slower case.
- picomatch's own suite (1983 tests at the 4.0.7 tag) passes on this copy.
- `scripts/picomatch-fork-check.sh`, run by the "picomatch fork" job in
  `.github/workflows/checks.yml`, runs both, and fails when picomatch
  publishes a release after 4.0.7, when a picomatch advisory appears that is
  not in its reviewed list, or when GHSA-fc5q-39xc-vjpf itself is missing
  from the list. npm audit and Dependabot do not look at a local package, so
  that job is what tells us to look at this copy again.

## Known limits

These still backtrack exponentially, here exactly as in 4.0.7:

| pattern | string | time |
|---|---|---|
| `+(*a\|b)` | `ba` x 16 + `!` | 4.3 s |
| `*(a*\|b)` | `a` x 22 + `/` | 0.2 s, x4 every 2 characters |
| `+(*a\|b*)` | `a` x 22 + `/` | 0.3 s, x4 every 2 characters |

- Several branches whose only open-ended parts are plain `*`s, unless all of
  them start, or all end, with one: making them safe would change matches
  picomatch's own tests rely on (`*(a*|b)` must match `ax`).
- A pattern that holds one of those, or anything the automaton cannot read
  (quotes, POSIX classes, very long or deep patterns), next to a repeated
  extglob the fork would fix: rule 3 keeps 4.0.7's regex for the whole
  pattern, so `+(*a)*(a*|b)` is not fixed.
- Plain regex groups with quantifiers, such as `(a+)+`, which picomatch
  passes through as they are.
- A malformed pattern whose closing parenthesis is escaped, such as
  `+(+([ab])+([ab])\)`: picomatch compiles it into a different group
  structure than the one it analyses. It compiles as in 4.0.7.

Elsewhere, outside the reach of an npm override: Playwright 1.58 bundles its
own picomatch 2.3.1 and 4.0.3 in `lib/utilsBundleImpl.js`, and vite 5 (the
installer) bundles picomatch 2 in its own chunks.

## The installer on picomatch 4

The installer's micromatch, anymatch and readdirp were written for picomatch
2, and now get this copy of 4.0.7. picomatch 2 used Windows separators on
Windows by itself; 4.x only does so when its options say so, so
`picomatch(glob)` without options no longer treats a backslash as a
separator. Only tailwind's CLI watcher would notice, through chokidar; the
installer runs tailwind as a PostCSS plugin (`postcss.config.js`), whose
content scanning hands fast-glob forward-slash paths. On Linux the
installer's build output is byte for byte the same as before.

## Retiring this copy

When picomatch publishes a release that fixes GHSA-fc5q-39xc-vjpf, remove
the `picomatch` devDependency and the override from packages/daw-ui and the
installer, delete this folder, and drop the "picomatch fork" job.
