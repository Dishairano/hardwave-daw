// The FPS meter counts React renders through the hook React looks for when
// react-dom loads. A plain script, before the app's module, so it is in
// place first whatever order the bundler gives the app's chunks. It keeps
// a count and nothing else: React's internals are not stored.
(function () {
  var w = window
  w.__hwCommits = 0
  var count = function () { w.__hwCommits++ }
  var hook = w.__REACT_DEVTOOLS_GLOBAL_HOOK__
  if (hook) {
    var previous = hook.onCommitFiberRoot
    hook.onCommitFiberRoot = function () {
      count()
      if (previous) return previous.apply(this, arguments)
    }
    return
  }
  w.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {
    supportsFiber: true,
    isDisabled: false,
    renderers: new Map(),
    inject: function () { return 1 },
    onCommitFiberRoot: count,
    onCommitFiberUnmount: function () {},
    onPostCommitFiberRoot: function () {},
    checkDCE: function () {},
  }
})()
