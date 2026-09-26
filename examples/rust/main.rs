// cdev watch examples/rust/main.rs      (compiles with rustc + re-runs on save)
#[macro_use]
mod cdev;

use std::collections::HashMap;

#[derive(Debug)]
struct ListNode {
    val: i32,
    next: Option<Box<ListNode>>,
}

fn reverse(mut head: Option<Box<ListNode>>) -> Option<Box<ListNode>> {
    let mut prev: Option<Box<ListNode>> = None;
    while let Some(mut node) = head {
        head = node.next.take();
        node.next = prev;
        prev = Some(node);
        cdev_w!("reversed", &prev);
    }
    prev
}

fn two_sum(nums: &[i32], target: i32) -> Option<(usize, usize)> {
    cdev_trace!(nums, target);
    let mut seen: HashMap<i32, usize> = HashMap::new();
    for (i, &n) in nums.iter().enumerate() {
        if let Some(&j) = seen.get(&(target - n)) {
            return Some((j, i));
        }
        seen.insert(n, i);
        cdev_w!("seen", &seen);
    }
    None
}

fn main() {
    let mut head = None;
    for v in (1..=4).rev() {
        head = Some(Box::new(ListNode { val: v, next: head }));
    }
    cdev_w!("list", &head);
    reverse(head);
    let r = cdev_w!(two_sum(&[2, 7, 11, 15], 9));
    let grid = cdev_w!(vec![vec![0u8; 3]; 3]);
    cdev_log!("answer {:?}, grid rows {}", r, grid.len());
}
