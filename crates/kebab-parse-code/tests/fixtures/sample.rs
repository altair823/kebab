//! sample fixture

use std::fmt;

const ANSWER: u32 = 42;

/// Doc comment on a free fn.
pub fn parse(input: &str) -> usize {
    input.len()
}

pub struct Foo {
    pub n: u32,
}

impl Foo {
    /// method doc
    pub fn double(&self) -> u32 {
        self.n * 2
    }

    fn name() -> &'static str {
        "foo"
    }
}

pub trait Greet {
    fn hello(&self) -> String;
}

mod inner {
    pub fn helper() -> bool {
        true
    }
}
