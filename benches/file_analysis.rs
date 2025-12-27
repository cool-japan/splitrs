//! Benchmarks for SplitRS file analysis and module generation
//!
//! This benchmark suite measures the performance of key operations:
//! - Parsing Rust source files
//! - Analyzing types and impl blocks
//! - Detecting method dependencies
//! - Generating module structure
//! - Code generation and formatting

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;

/// Benchmark parsing small Rust files (< 100 lines)
fn bench_parse_small_file(c: &mut Criterion) {
    let code = r#"
        struct User {
            name: String,
            age: u32,
        }

        impl User {
            fn new(name: String, age: u32) -> Self {
                Self { name, age }
            }

            fn get_name(&self) -> &str {
                &self.name
            }

            fn get_age(&self) -> u32 {
                self.age
            }
        }
    "#;

    c.bench_function("parse_small_file", |b| {
        b.iter(|| {
            let result = syn::parse_file(black_box(code));
            black_box(result)
        })
    });
}

/// Benchmark parsing medium Rust files (100-500 lines)
fn bench_parse_medium_file(c: &mut Criterion) {
    // Generate a medium-sized file with multiple types
    let mut code = String::from("use std::collections::HashMap;\n\n");

    for i in 0..10 {
        code.push_str(&format!(
            r#"
struct Type{} {{
    field1: String,
    field2: i32,
    field3: HashMap<String, Vec<u8>>,
}}

impl Type{} {{
    fn new(field1: String, field2: i32) -> Self {{
        Self {{ field1, field2, field3: HashMap::new() }}
    }}

    fn method1(&self) -> &str {{
        &self.field1
    }}

    fn method2(&self) -> i32 {{
        self.field2
    }}

    fn method3(&mut self, key: String, value: Vec<u8>) {{
        self.field3.insert(key, value);
    }}

    fn method4(&self, key: &str) -> Option<&Vec<u8>> {{
        self.field3.get(key)
    }}
}}
"#,
            i, i
        ));
    }

    let code_len = code.len();

    let mut group = c.benchmark_group("parse_medium_file");
    group.throughput(Throughput::Bytes(code_len as u64));

    group.bench_function("medium_file", |b| {
        b.iter(|| {
            let result = syn::parse_file(black_box(&code));
            black_box(result)
        })
    });

    group.finish();
}

/// Benchmark parsing large Rust files (500-2000 lines)
fn bench_parse_large_file(c: &mut Criterion) {
    // Generate a large file with many types and methods
    let mut code = String::from("use std::collections::{HashMap, HashSet};\n\n");

    for i in 0..50 {
        code.push_str(&format!(
            r#"
struct LargeType{} {{
    data: Vec<u8>,
    metadata: HashMap<String, String>,
    cache: HashSet<String>,
}}

impl LargeType{} {{
    fn new() -> Self {{
        Self {{
            data: Vec::new(),
            metadata: HashMap::new(),
            cache: HashSet::new(),
        }}
    }}

    fn insert_data(&mut self, value: u8) {{
        self.data.push(value);
    }}

    fn set_metadata(&mut self, key: String, value: String) {{
        self.metadata.insert(key, value);
    }}

    fn add_to_cache(&mut self, key: String) {{
        self.cache.insert(key);
    }}

    fn get_data(&self) -> &[u8] {{
        &self.data
    }}

    fn get_metadata(&self, key: &str) -> Option<&String> {{
        self.metadata.get(key)
    }}

    fn has_cached(&self, key: &str) -> bool {{
        self.cache.contains(key)
    }}

    fn clear(&mut self) {{
        self.data.clear();
        self.metadata.clear();
        self.cache.clear();
    }}

    fn size(&self) -> usize {{
        self.data.len()
    }}
}}
"#,
            i, i
        ));
    }

    let code_len = code.len();

    let mut group = c.benchmark_group("parse_large_file");
    group.throughput(Throughput::Bytes(code_len as u64));

    group.bench_function("large_file", |b| {
        b.iter(|| {
            let result = syn::parse_file(black_box(&code));
            black_box(result)
        })
    });

    group.finish();
}

/// Benchmark code formatting with prettyplease
fn bench_code_formatting(c: &mut Criterion) {
    let code = r#"
        struct User {
            name: String,
            age: u32,
        }

        impl User {
            fn new(name: String, age: u32) -> Self {
                Self { name, age }
            }
        }
    "#;

    let file = syn::parse_file(code).unwrap();

    c.bench_function("code_formatting", |b| {
        b.iter(|| {
            let formatted = prettyplease::unparse(black_box(&file));
            black_box(formatted)
        })
    });
}

/// Benchmark generic type parsing (with complex generics)
fn bench_parse_generic_types(c: &mut Criterion) {
    let code = r#"
        struct Container<T, U, V>
        where
            T: Clone + Send + Sync,
            U: Default,
            V: std::fmt::Debug,
        {
            data: Vec<T>,
            metadata: U,
            debug_info: V,
        }

        impl<T, U, V> Container<T, U, V>
        where
            T: Clone + Send + Sync,
            U: Default,
            V: std::fmt::Debug,
        {
            fn new(data: Vec<T>, metadata: U, debug_info: V) -> Self {
                Self { data, metadata, debug_info }
            }

            fn clone_data(&self) -> Vec<T>
            where
                T: Clone,
            {
                self.data.clone()
            }

            fn get_metadata(&self) -> &U {
                &self.metadata
            }

            fn debug(&self) -> String
            where
                V: std::fmt::Debug,
            {
                format!("{:?}", self.debug_info)
            }
        }
    "#;

    c.bench_function("parse_generic_types", |b| {
        b.iter(|| {
            let result = syn::parse_file(black_box(code));
            black_box(result)
        })
    });
}

/// Benchmark parsing with various file sizes
fn bench_scalability(c: &mut Criterion) {
    let mut group = c.benchmark_group("scalability");

    for size in [10, 50, 100, 200, 500].iter() {
        let mut code = String::from("use std::collections::HashMap;\n\n");

        for i in 0..*size {
            code.push_str(&format!(
                r#"
struct Type{} {{
    field: i32,
}}

impl Type{} {{
    fn new() -> Self {{ Self {{ field: 0 }} }}
    fn get(&self) -> i32 {{ self.field }}
}}
"#,
                i, i
            ));
        }

        let code_len = code.len();
        group.throughput(Throughput::Bytes(code_len as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, _| {
            b.iter(|| {
                let result = syn::parse_file(black_box(&code));
                black_box(result)
            })
        });
    }

    group.finish();
}

/// Benchmark trait implementation parsing
fn bench_parse_trait_impls(c: &mut Criterion) {
    let code = r#"
        use std::fmt::{Debug, Display};

        struct User {
            name: String,
            age: u32,
        }

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
                write!(f, "{} ({})", self.name, self.age)
            }
        }

        impl Clone for User {
            fn clone(&self) -> Self {
                Self {
                    name: self.name.clone(),
                    age: self.age,
                }
            }
        }

        impl Default for User {
            fn default() -> Self {
                Self {
                    name: String::new(),
                    age: 0,
                }
            }
        }
    "#;

    c.bench_function("parse_trait_impls", |b| {
        b.iter(|| {
            let result = syn::parse_file(black_box(code));
            black_box(result)
        })
    });
}

criterion_group!(
    benches,
    bench_parse_small_file,
    bench_parse_medium_file,
    bench_parse_large_file,
    bench_code_formatting,
    bench_parse_generic_types,
    bench_scalability,
    bench_parse_trait_impls,
);

criterion_main!(benches);
