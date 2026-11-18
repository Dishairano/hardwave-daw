'use strict';

// picomatch's test suite is written for mocha's globals. node:test has the
// same describe and it, so the suite runs on Node's own test runner and
// needs no test framework from npm.
const test = require('node:test');

Object.assign(global, {
  describe: test.describe,
  it: test.it,
  before: test.before,
  after: test.after,
  beforeEach: test.beforeEach,
  afterEach: test.afterEach
});
