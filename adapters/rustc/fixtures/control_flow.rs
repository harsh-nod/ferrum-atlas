#![allow(dead_code)]

pub fn branch(flag: bool) -> u8 {
    if flag { 11 } else { 22 }
}

pub fn return_value(value: u32) -> u32 {
    value
}

pub struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {}
}

pub fn with_drop(callback: fn()) {
    let first = Guard;
    let second = Guard;
    callback();
    let _ = (&first, &second);
}

pub fn direct_call(value: u32) -> u32 {
    return_value(value)
}

pub unsafe fn pointer_write(pointer: *mut u8, value: u8) {
    unsafe { *pointer = value; }
}

pub fn divide(left: u32, right: u32) -> u32 {
    left / right
}
