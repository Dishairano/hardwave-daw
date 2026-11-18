// The part of picomatch's API the DAW's own tests use. vite, vitest,
// tinyglobby and fdir load this package as plain JavaScript.

declare namespace picomatch {
  interface PicomatchOptions {
    [option: string]: unknown
  }

  type Matcher = (test: string) => boolean

  function makeRe(glob: string, options?: PicomatchOptions): RegExp
  function isMatch(str: string | string[], patterns: string | string[], options?: PicomatchOptions): boolean
}

declare function picomatch(glob: string | string[], options?: picomatch.PicomatchOptions): picomatch.Matcher

export = picomatch
