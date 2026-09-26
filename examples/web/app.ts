// Served by `cdev watch` (compiled with bun). `cdev` is the global from /cdev.js.
declare const cdev: { w<T>(v: T, label?: string): T; trace<F>(f: F, name?: string): F };

type Grid = number[][];

// dynamic programming: number of paths through a grid with walls (1 = wall)
const countPaths = cdev.trace(function countPaths(walls: Grid): number {
  const rows = walls.length, cols = walls[0].length;
  const dp: Grid = Array.from({ length: rows }, () => Array(cols).fill(0));
  dp[0][0] = 1;
  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      if (walls[r][c]) { dp[r][c] = 0; continue; }
      if (r > 0) dp[r][c] += dp[r - 1][c];
      if (c > 0) dp[r][c] += dp[r][c - 1];
      cdev.w(dp, 'dp');
    }
  }
  return dp[rows - 1][cols - 1];
}, 'countPaths');

document.getElementById('go')!.addEventListener('click', async () => {
  const walls: Grid = [
    [0, 0, 0, 0],
    [0, 1, 0, 0],
    [0, 0, 0, 1],
    [1, 0, 0, 0],
  ];
  const n = countPaths(walls);
  const todo = await fetch('https://jsonplaceholder.typicode.com/todos/1').then((r) => r.json()).catch(() => null);
  console.log('paths:', n, 'todo:', todo);
  document.getElementById('out')!.textContent = `paths: ${n}`;
});
