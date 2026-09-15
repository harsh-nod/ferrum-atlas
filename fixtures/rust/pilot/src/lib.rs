pub mod engine;

pub fn entry(value: u32) -> u32 {
    engine::step(value)
}

pub fn callback(value: u32, operation: fn(u32) -> u32) -> u32 {
    operation(value)
}
