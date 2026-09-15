pub fn step(value: u32) -> u32 {
    if value == 0 {
        return 0;
    }
    leaf(value)
}

fn leaf(value: u32) -> u32 {
    math::double(value + 1)
}
