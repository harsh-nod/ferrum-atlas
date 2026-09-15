#![allow(dead_code)]

mod left {
    pub fn same() -> u32 { 2 }
}

mod right {
    pub fn same() -> u32 { 5 }
}

use left::same as selected;

struct Engine;
impl Engine {
    fn step(&self) -> u32 { 11 }
}

pub fn entry() -> u32 {
    selected() + right::same() + Engine.step()
}

fn callback_boundary(pointer: fn() -> u32) -> u32 {
    pointer()
}

fn recursive(depth: u32) -> u32 {
    if depth == 0 { 1 } else { recursive(depth - 1) }
}

#[test]
fn independent_compiler_runtime_controls() {
    assert_eq!(left::same(), 2);
    assert_eq!(right::same(), 5);
    assert_eq!(entry(), 18);
    assert_eq!(callback_boundary(left::same), 2);
    assert_eq!(recursive(2), 1);
}
