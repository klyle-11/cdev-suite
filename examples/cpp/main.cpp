// cdev watch examples/cpp/main.cpp      (compiles + re-runs on save)
#include "cdev.hpp"
#include <map>
#include <string>
#include <vector>

int partition(std::vector<int>& a, int lo, int hi) {
  CDEV_TRACE(lo, hi);
  int pivot = a[hi], i = lo;
  for (int j = lo; j < hi; ++j) {
    if (a[j] < pivot) {
      std::swap(a[i], a[j]);
      ++i;
      CDEV_WATCH("quicksort", a);
    }
  }
  std::swap(a[i], a[hi]);
  CDEV_WATCH("quicksort", a);
  return i;
}

void quicksort(std::vector<int>& a, int lo, int hi) {
  CDEV_TRACE(lo, hi);
  if (lo >= hi) return;
  int p = partition(a, lo, hi);
  quicksort(a, lo, p - 1);
  quicksort(a, p + 1, hi);
}

int main() {
  std::vector<int> a{9, 4, 7, 1, 8, 2, 6};
  quicksort(a, 0, static_cast<int>(a.size()) - 1);
  std::vector<std::vector<int>> grid(4, std::vector<int>(4, 0));
  for (int i = 0; i < 4; ++i) { grid[i][i] = 1; CDEV_W(grid); }
  std::map<std::string, int> counts{{"apples", 3}, {"pears", 7}};
  CDEV_W(counts);
  CDEV_LOG("sorted ", a.size(), " items");
}
