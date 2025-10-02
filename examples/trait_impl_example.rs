//! Example file with trait implementations to test SplitRS trait support

use std::fmt::{Debug, Display};

/// A simple user struct
pub struct User {
    pub name: String,
    pub age: u32,
}

impl User {
    pub fn new(name: String, age: u32) -> Self {
        Self { name, age }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }
}

// Trait implementations
impl Debug for User {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("User")
            .field("name", &self.name)
            .field("age", &self.age)
            .finish()
    }
}

impl Display for User {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "User {{ name: {}, age: {} }}", self.name, self.age)
    }
}

impl Clone for User {
    fn clone(&self) -> Self {
        User {
            name: self.name.clone(),
            age: self.age,
        }
    }
}

impl Default for User {
    fn default() -> Self {
        User {
            name: String::from("Anonymous"),
            age: 0,
        }
    }
}

/// Another struct to test multiple types
pub struct Product {
    pub id: u64,
    pub name: String,
    pub price: f64,
}

impl Product {
    pub fn new(id: u64, name: String, price: f64) -> Self {
        Self { id, name, price }
    }
}

impl Display for Product {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Product #{}: {} (${:.2})",
            self.id, self.name, self.price
        )
    }
}

impl Debug for Product {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Product")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("price", &self.price)
            .finish()
    }
}
