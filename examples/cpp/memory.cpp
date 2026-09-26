// cdev watch examples/cpp/memory.cpp   → open the Mem tab (5)
#include "cdev.hpp"
#include <memory>
#include <string>
#include <vector>

struct Node {
  int val;
  Node* next;
};

int main() {
  // a vector's buffer lives on the heap; watch it move when capacity runs out
  std::vector<int> v;
  for (int i = 1; i <= 9; ++i) {
    v.push_back(i * 10);
    CDEV_WATCH("v", v);
  }

  // short strings live inside the object (small-string optimisation); long ones go to the heap
  std::string small = "hi";
  std::string big = "this string is long enough to need a heap allocation";
  CDEV_W(small);
  CDEV_W(big);

  // raw pointers: one to a stack variable, one to a heap node
  int answer = 42;
  int* p = &answer;
  CDEV_W(answer);
  CDEV_W(p);
  Node* head = new Node{1, new Node{2, nullptr}};
  CDEV_W(head);
  CDEV_WATCH("head->next", head->next);

  // smart pointers: ownership and reference counts
  auto owned = std::make_unique<int>(7);
  CDEV_W(owned);
  auto shared = std::make_shared<std::string>("shared");
  auto copy = shared;
  CDEV_W(shared);
  CDEV_W(copy);

  delete head->next;
  delete head;
}
