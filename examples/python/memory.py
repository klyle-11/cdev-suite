# cdev watch examples/python/memory.py   → open the Mem tab (5)
import cdev

# names point at objects; id() is the object's address in CPython
a = [1, 2, 3]
b = a                     # same list, two names
cdev.w(a, "a")
cdev.w(b, "b")
b.append(4)               # "changing b" changes a too
cdev.w(a, "a")

# the classic aliasing bug: one row object, three references
grid = [[0] * 3] * 3
cdev.w(grid, "grid")
grid[0][0] = 1            # every row "changes"
cdev.w(grid, "grid")

fixed = [[0] * 3 for _ in range(3)]   # three separate rows
cdev.w(fixed, "fixed")
