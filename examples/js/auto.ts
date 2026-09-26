// cdev --auto watch examples/js/auto.ts     (no cdev calls: every local is recorded per statement)
function bubbleSort(arr: number[]): number[] {
  const n = arr.length;
  for (let i = 0; i < n; i++) {
    for (let j = 0; j < n - i - 1; j++) {
      if (arr[j] > arr[j + 1]) [arr[j], arr[j + 1]] = [arr[j + 1], arr[j]];
    }
  }
  return arr;
}

class Stack {
  constructor() { this.items = []; }
  items: number[] = [];
  push(x: number) { const size = this.items.push(x); return size; }
}

const words = ['b', 'a', 'c'];
const lengths = words.map((w) => { const len = w.length * 2; return len; });
bubbleSort([5, 1, 4, 2, 8]);
const s = new Stack();
s.push(1); s.push(2);
