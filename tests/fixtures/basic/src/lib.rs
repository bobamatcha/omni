/// Adds two numbers together.
pub fn add_numbers(a: i32, b: i32) -> i32 {
    a + b
}

/// Subtract two numbers.
pub fn subtract_numbers(a: i32, b: i32) -> i32 {
    a - b
}

pub struct MathThing {
    value: i32,
}

impl MathThing {
    pub fn value(&self) -> i32 {
        self.value
    }
}
