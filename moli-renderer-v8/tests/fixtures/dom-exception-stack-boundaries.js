function domExceptionStackBoundariesProbe(realm) {
  const rows = [], failures = [];
  const check = (name, actual, expected) => {
    rows.push({name, actual});
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({name, actual, expected});
  };
  const entries = [
    ['DOMException', ['message', 'AbortError']],
    ['QuotaExceededError', ['full', {quota: 10, requested: 12}]],
    ['WebSocketError', ['closed']],
  ];
  if (realm.document) entries.push(['RTCError', [{errorDetail: 'dtls-failure'}, 'rtc']]);
  const descriptor = Object.getOwnPropertyDescriptor(realm.Error, 'captureStackTrace');
  let captureReads = 0;
  Object.defineProperty(realm.Error, 'captureStackTrace', {configurable: true, get() {captureReads++; throw new Error('author capture lookup');}});
  try {
    for (const [name, args] of entries) {
      check(name + ':exposure', typeof realm[name], 'function');
      if (typeof realm[name] !== 'function') continue;
      const error = Reflect.construct(realm[name], args);
      check(name + ':script-stack', [Object.hasOwn(error, 'stack'), typeof error.stack], [false, 'undefined']);
      const Derived = class extends realm[name] {};
      let stackReads = 0;
      Object.defineProperty(Derived.prototype, 'stack', {configurable: true, get() {stackReads++; return 'inherited stack';}});
      const derived = Reflect.construct(realm[name], args, Derived);
      check(name + ':subclass-initialization', [Object.getPrototypeOf(derived) === Derived.prototype, Object.hasOwn(derived, 'stack'), stackReads], [true, false, 0]);
      check(name + ':inherited-stack', [derived.stack, stackReads], ['inherited stack', 1]);
    }
    let cloneError;
    try {realm.structuredClone(Symbol('not cloneable'));} catch (error) {cloneError = error;}
    const stackDescriptor = Object.getOwnPropertyDescriptor(cloneError, 'stack');
    check('native-clone-error', [cloneError instanceof realm.DOMException, cloneError.name, typeof cloneError.stack], [true, 'DataCloneError', 'string']);
    check('native-stack-descriptor', [!!stackDescriptor, stackDescriptor?.enumerable, stackDescriptor?.configurable], [true, false, true]);
    if (realm.document) {
      let selectorError;
      try {realm.document.querySelector('[');} catch (error) {selectorError = error;}
      check('native-selector-error', [selectorError instanceof realm.DOMException, selectorError.name, typeof selectorError.stack], [true, 'SyntaxError', 'string']);
    }
    check('author-capture-reads', captureReads, 0);
  } finally {
    if (descriptor) Object.defineProperty(realm.Error, 'captureStackTrace', descriptor);
    else delete realm.Error.captureStackTrace;
  }
  return {rows, failures};
}
