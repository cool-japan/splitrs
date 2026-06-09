use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// Example large struct for demonstrating splitrs
pub struct DataStore {
    data: HashMap<String, Vec<u8>>,
    cache: HashSet<String>,
    connections: Arc<Mutex<Vec<Connection>>>,
    config: Config,
    metrics: Metrics,
}

pub struct Connection {
    id: usize,
    active: bool,
}

pub struct Config {
    max_connections: usize,
    timeout_ms: u64,
}

pub struct Metrics {
    requests: u64,
    errors: u64,
}

impl DataStore {
    /// Constructor
    pub fn new(config: Config) -> Self {
        Self {
            data: HashMap::new(),
            cache: HashSet::new(),
            connections: Arc::new(Mutex::new(Vec::new())),
            config,
            metrics: Metrics {
                requests: 0,
                errors: 0,
            },
        }
    }

    /// Insert data
    pub fn insert(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        if self.cache.contains(&key) {
            return Err("Key already exists in cache".to_string());
        }
        self.data.insert(key.clone(), value);
        self.cache.insert(key);
        self.metrics.requests += 1;
        Ok(())
    }

    /// Get data
    pub fn get(&self, key: &str) -> Option<&Vec<u8>> {
        self.data.get(key)
    }

    /// Remove data
    pub fn remove(&mut self, key: &str) -> Option<Vec<u8>> {
        self.cache.remove(key);
        self.data.remove(key)
    }

    /// Clear all data
    pub fn clear(&mut self) {
        self.data.clear();
        self.cache.clear();
        self.metrics.requests = 0;
        self.metrics.errors = 0;
    }

    /// Get connection count
    pub fn connection_count(&self) -> usize {
        self.connections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    /// Add connection
    pub fn add_connection(&self, conn: Connection) {
        self.connections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(conn);
    }

    /// Get metrics
    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    /// Get config
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Contains key
    pub fn contains_key(&self, key: &str) -> bool {
        self.data.contains_key(key)
    }

    /// Keys
    pub fn keys(&self) -> Vec<String> {
        self.data.keys().cloned().collect()
    }

    /// Size
    pub fn size(&self) -> usize {
        self.data.len()
    }
}

fn main() {
    // This is an example file demonstrating a large struct with many methods
    // To split this file, run:
    // splitrs -i examples/large_struct.rs -o output/ --split-impl-blocks --max-impl-lines 300

    let config = Config {
        max_connections: 100,
        timeout_ms: 5000,
    };

    println!(
        "Config: max_connections={}, timeout_ms={}",
        config.max_connections, config.timeout_ms
    );

    let mut store = DataStore::new(config);

    let _ = store.insert("key1".to_string(), vec![1, 2, 3]);
    println!("DataStore size: {}", store.size());
    println!("Contains key1: {}", store.contains_key("key1"));

    let conn = Connection {
        id: 1,
        active: true,
    };
    println!("Adding connection id={}, active={}", conn.id, conn.active);
    store.add_connection(conn);
    println!("Connection count: {}", store.connection_count());
}
