// cdev run -- node examples/node-api/server.js
// then: curl localhost:3000/api/users/2   (or open it in a browser)
const http = require('http');

const users = [
  { id: 1, name: 'Ada', tags: ['math'] },
  { id: 2, name: 'Linus', tags: ['kernel', 'git'] },
];

const findUser = cdev.trace(function findUser(id) {
  return users.find((u) => u.id === id) ?? null;
});

const loadProfile = cdev.trace(async function loadProfile(user) {
  const res = await fetch(`http://localhost:3000/api/profile?name=${user.name}`);
  return res.json();
});

// algorithm demo: every snapshot of {arr, i, j} becomes a step you can replay
function bubbleSort(input) {
  const arr = [...input];
  for (let i = 0; i < arr.length; i++) {
    for (let j = 0; j < arr.length - i - 1; j++) {
      if (arr[j] > arr[j + 1]) {
        [arr[j], arr[j + 1]] = [arr[j + 1], arr[j]];
        cdev.w({ arr, i, j }, 'bubbleSort');
      }
    }
  }
  return arr;
}

class ListNode { constructor(val, next = null) { this.val = val; this.next = next; } }
class TreeNode { constructor(val) { this.val = val; this.left = null; this.right = null; } }
function insert(root, val) {
  if (!root) return new TreeNode(val);
  if (val < root.val) root.left = insert(root.left, val); else root.right = insert(root.right, val);
  return root;
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, 'http://x');
  if (url.pathname.startsWith('/api/users/')) {
    const id = cdev.w(Number(url.pathname.split('/').pop()));
    const user = findUser(id);
    if (!user) { res.writeHead(404, { 'content-type': 'application/json' }); return res.end('{"error":"not found"}'); }
    const profile = await loadProfile(user);
    console.log('served user', user.name);
    res.writeHead(200, { 'content-type': 'application/json' });
    return res.end(JSON.stringify({ ...user, profile }));
  }
  if (url.pathname === '/api/profile') {
    res.writeHead(200, { 'content-type': 'application/json' });
    return res.end(JSON.stringify({ bio: `${url.searchParams.get('name')} writes code`, followers: 42 }));
  }
  if (url.pathname === '/api/demo') {
    bubbleSort([5, 1, 4, 2, 8, 3]);
    let list = null;
    for (const v of [4, 3, 2, 1]) { list = new ListNode(v, list); cdev.w(list, 'linkedList'); }
    let root = null;
    for (const v of [8, 3, 10, 1, 6, 14, 4]) { root = insert(root, v); cdev.w(root, 'bst'); }
    cdev.w({ A: ['B', 'C'], B: ['A', 'D'], C: ['A', 'D'], D: ['B', 'C'] }, 'graph');
    cdev.w(new Map([['apples', 3], ['pears', 7]]), 'inventory');
    res.writeHead(200, { 'content-type': 'text/plain' });
    return res.end('ok');
  }
  res.writeHead(404); res.end();
});

server.listen(3000, () => console.log('listening on http://localhost:3000'));
