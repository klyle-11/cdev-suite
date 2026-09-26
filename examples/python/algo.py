# cdev watch examples/python/algo.py      (re-runs on save)
from collections import deque
import logging
import cdev

logging.basicConfig(level=logging.INFO)
log = logging.getLogger("algo")

graph = {"A": ["B", "C"], "B": ["D"], "C": ["D", "E"], "D": ["F"], "E": ["F"], "F": []}
cdev.w(graph, "graph")


@cdev.trace
def bfs(start: str, goal: str):
    queue = deque([[start]])
    visited = set()
    while queue:
        cdev.w(list(queue), "queue")
        path = queue.popleft()
        node = path[-1]
        if node == goal:
            return path
        if node in visited:
            continue
        visited.add(node)
        cdev.w(visited, "visited")
        for nxt in graph[node]:
            queue.append(path + [nxt])
    return None


class Node:
    def __init__(self, val, left=None, right=None):
        self.val, self.left, self.right = val, left, right


@cdev.trace
def heap_push(heap: list, x: int):
    heap.append(x)
    i = len(heap) - 1
    while i > 0 and heap[(i - 1) // 2] > heap[i]:
        p = (i - 1) // 2
        heap[i], heap[p] = heap[p], heap[i]
        i = p
        cdev.w(list(heap), "minHeap")
    return heap


path = bfs("A", "F")
log.info("shortest path %s", path)
h = []
for x in [7, 3, 9, 1, 5, 2]:
    heap_push(h, x)
cdev.w(Node(2, Node(1), Node(3)), "tree")
