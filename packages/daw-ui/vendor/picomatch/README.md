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
`index.d.ts`, `package.json` and `test/`. The other files are byte for byte
the published 4.0.7.

For a repeated extglob `+(...)` or `*(...)`:

1. One branch with a top-level `*` that cannot match a slash (`+(*a)`,
   `+(a*b)`): repeating it matches nothing one match does not, so `+(B)`
   compiles as `@(B)` and `*(B)` as `?(B)`. Same matches, no nested
   quantifier. The same holds for one negation, `+(!(x))`, and for several
   branches that all start, or all end, with such a `*` (`+(*.js|*.ts)`).
2. Any other single branch is read into a small automaton that counts the
   ways a backtracking engine can match the same text. If repeating it can
   take exponential time (`+(*(a)a)`, `+(a+)`, `+(*[a-z])`) the extglob is
   matched literally, as upstream already does with its other risky repeated
   extglobs. If not (`*(*(of*(o)x)o)`, `+(a+b)`, `+(*/)`), it compiles as
   upstream compiles it. A body the model does not read (a negation inside
   it, braces, quotes, POSIX classes) is matched literally whenever it holds
   an open-ended quantifier.
3. `maxExtglobRecursion: false` turns all of this off, and a raised
   `maxExtglobRecursion` keeps nested repeated extglobs as upstream compiles
   them, as upstream documents.

## Tests

- `node --test test/`: the advisory's patterns must answer within 100 ms
  (each takes seconds on 4.0.7: run `PICOMATCH=<path to 4.0.7> node --test
  test/` to see it), and every rewritten or kept pattern must match exactly
  what upstream's compile matches, on all strings up to five characters.
- picomatch's own suite (1983 tests at the 4.0.7 tag) passes on this copy.
- `scripts/picomatch-fork-check.sh`, run by the "picomatch fork" job in
  `.github/workflows/checks.yml`, runs both, and fails when picomatch
  publishes a release after 4.0.7 or a picomatch advisory appears that is
  not in its reviewed list. npm audit and Dependabot do not look at a local
  package, so that job is what tells us to look at this copy again.

## Known limits

These still backtrack exponentially, here as in 4.0.7. They are outside the
advisory, and making them safe would change matches picomatch's own tests
rely on (`*(a*|b)` must match `ax`):

| pattern | string | time |
|---|---|---|
| `+(*a\|b)` | `ba` x 16 + `!` | 4.5 s |
| `*(a*\|b)` | `a` x 22 + `/` | 0.2 s, x4 every 2 characters |
| `+(*a\|b*)` | `a` x 22 + `/` | 0.3 s, x4 every 2 characters |

Several branches where some, but not all, have a top-level `*`, or where the
stars sit at different ends. Also a malformed pattern whose closing
parenthesis is escaped (`+(!(a)@(a)?+([ab])\)`), which picomatch compiles
into a different group structure than the one it analyses.

Elsewhere, outside the reach of an npm override: Playwright 1.58 bundles its
own picomatch 2.3.1 and 4.0.3 in `lib/utilsBundleImpl.js`, and vite 5 (the
installer) bundles picomatch 2 in its own chunks.

## Retiring this copy

When picomatch publishes a release that fixes GHSA-fc5q-39xc-vjpf, remove
the `picomatch` devDependency and the override from packages/daw-ui and the
installer, delete this folder, and drop the "picomatch fork" job.
