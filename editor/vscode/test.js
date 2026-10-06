// node test.js — checks the formatting used for inline values
const assert = require('assert');
const { lineText, short, fnMarkdown } = require('./format');
assert.strictEqual(lineText({ name: 'sort()', v: { arr: [1, 2], i: 0 } }), 'arr=[1,2]  i=0');
assert.strictEqual(lineText({ name: 'sort()', v: { arr: [1, 2], i: 1 } }, { arr: [1, 2], i: 0 }), 'i=1');
assert.strictEqual(lineText({ name: 'sort.arr', auto: true, v: [2, 1] }), 'arr = [2,1]');
assert.strictEqual(lineText({ name: 'total', v: 42 }), 'total = 42');
assert.strictEqual(short({ __class: 'Node', val: 1, __id: 3 }), '{"val":1}');
assert.strictEqual(short({ __t: 'undefined' }), 'undefined');
assert.strictEqual(
  fnMarkdown({ signature: 'swap(a, i, j)', line: 3, end: 5, summary: 'Takes a, i, j. Changes a.', rows: [['changes', 'a'], ['locals', 't = `x`']] }),
  "**cdev · static** `swap(a, i, j)` · lines 3–5\n\nTakes a, i, j. Changes a.\n\n- **changes** `a`\n- **locals** `t = 'x'`",
);
console.log('format tests ok');
