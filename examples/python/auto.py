# cdev --auto watch examples/python/auto.py
# No cdev calls at all: every function's locals are recorded after each line.
# Vars tab: bubble_sort() draws arr with ↑i ↑j; step with ←/→.


def bubble_sort(arr):
    n = len(arr)
    for i in range(n):
        for j in range(n - i - 1):
            if arr[j] > arr[j + 1]:
                arr[j], arr[j + 1] = arr[j + 1], arr[j]
    return arr


class Node:
    def __init__(self, val):
        self.val = val
        self.left = None
        self.right = None


def insert(root, val):
    if root is None:
        return Node(val)
    if val < root.val:
        root.left = insert(root.left, val)
    else:
        root.right = insert(root.right, val)
    return root


def build_tree(values):
    root = None
    for v in values:
        root = insert(root, v)
    return root


bubble_sort([5, 1, 4, 2, 8])
tree = build_tree([8, 3, 10, 1, 6])
