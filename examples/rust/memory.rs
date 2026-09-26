// cdev watch examples/rust/memory.rs   → open the Mem tab (5)
#[macro_use]
mod cdev;

use std::rc::Rc;

#[derive(Debug)]
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    // Vec = (ptr, len, cap) on the stack + a buffer on the heap that moves when it grows
    let mut v: Vec<u64> = Vec::new();
    for i in 0..9 {
        v.push(i);
        cdev_w!("v", &v);
    }

    // a Point lives on the stack; Box moves it to the heap
    let p = Point { x: 1, y: 2 };
    cdev_w!("p", &p);
    let boxed = Box::new(p); // `p` is moved: it no longer exists at the old address
    cdev_w!("boxed", &boxed);

    // Rc: shared ownership, one heap value, a reference count
    let a = Rc::new(String::from("shared"));
    let b = Rc::clone(&a);
    cdev_w!("a", &a);
    cdev_w!("b", &b);

    // String: empty ones don't allocate; pushing allocates then reallocates
    let mut s = String::new();
    cdev_w!("s", &s);
    for word in ["hello", " memory", " world!!"] {
        s.push_str(word);
        cdev_w!("s", &s);
    }
}
