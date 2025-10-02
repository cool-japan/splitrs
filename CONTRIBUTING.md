# Contributing to SplitRS

Thank you for your interest in contributing to SplitRS! This document provides guidelines for contributing to the project.

## Code of Conduct

Be respectful and inclusive. We welcome contributions from everyone.

## How to Contribute

### Reporting Bugs

1. Check if the bug has already been reported in [Issues](https://github.com/cool-japan/splitrs/issues)
2. If not, create a new issue with:
   - Clear title
   - Steps to reproduce
   - Expected behavior
   - Actual behavior
   - System information (OS, Rust version)
   - Example code (if applicable)

### Suggesting Features

1. Check [Issues](https://github.com/cool-japan/splitrs/issues) for existing feature requests
2. Create a new issue with:
   - Clear description of the feature
   - Use cases
   - Example usage
   - Any implementation ideas

### Pull Requests

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes
4. Add tests for new functionality
5. Ensure all tests pass (`cargo test`)
6. Ensure code is formatted (`cargo fmt`)
7. Ensure no clippy warnings (`cargo clippy -- -D warnings`)
8. Commit your changes (`git commit -m 'Add amazing feature'`)
9. Push to the branch (`git push origin feature/amazing-feature`)
10. Open a Pull Request

## Development Setup

```bash
# Clone the repository
git clone https://github.com/cool-japan/splitrs
cd splitrs

# Build
cargo build

# Run tests
cargo test

# Format code
cargo fmt

# Check with clippy
cargo clippy -- -D warnings

# Test on example
cargo run -- --input examples/large_struct.rs --output /tmp/test
```

## Project Structure

```
splitrs/
├── src/
│   ├── main.rs              # CLI and FileAnalyzer
│   ├── scope_analyzer.rs    # Organization strategy determination
│   ├── import_analyzer.rs   # Import generation
│   └── method_analyzer.rs   # Method extraction and clustering
├── examples/                 # Example files for testing
├── tests/                    # Integration tests
└── README.md
```

## Coding Guidelines

### Rust Style

- Follow Rust naming conventions (snake_case for variables, PascalCase for types)
- Use `cargo fmt` for formatting
- Address all `cargo clippy` warnings
- Write documentation for public APIs
- Add `#[doc]` examples where appropriate

### Code Quality

- Write clear, self-documenting code
- Add comments for complex algorithms
- Keep functions focused and small
- Avoid unnecessary complexity
- Use meaningful variable names

### Testing

- Add unit tests for new functions
- Add integration tests for new features
- Test edge cases
- Ensure all tests pass before submitting PR

### Documentation

- Update README.md if adding user-facing features
- Update inline documentation
- Add examples for new functionality
- Keep changelog updated (CHANGELOG.md)

## Priority Features (Roadmap)

### Current (v0.1.0) - 80% Complete
- ✅ AST parsing
- ✅ Method clustering
- ✅ Import generation
- ✅ Field visibility inference
- ✅ Complex type support

### Next Release (v0.2.0) - 90% Target
Priority order:
1. Type alias resolution
2. Configuration file support
3. Dependency graph visualization
4. Circular dependency detection

### Future (v1.0.0) - 100% Target
1. Cross-crate analysis
2. Refactoring preview mode
3. Undo/rollback support
4. Plugin system

## Testing

### Unit Tests

```bash
cargo test --lib
```

### Integration Tests

```bash
cargo test --test integration
```

### Example Testing

```bash
# Test on example file
cargo run -- --input examples/large_struct.rs --output /tmp/test

# Verify output compiles
cd /tmp/test
cargo init --lib
# Copy generated files to src/
cargo build
```

## Release Process

1. Update version in `Cargo.toml`
2. Update `CHANGELOG.md`
3. Commit: `git commit -m "Release v0.x.0"`
4. Tag: `git tag v0.x.0`
5. Push: `git push && git push --tags`
6. Publish: `cargo publish`

## License

By contributing, you agree that your contributions will be licensed under the same license as the project (MIT OR Apache-2.0).

## Questions?

Feel free to ask questions by:
- Opening an issue
- Starting a discussion
- Contacting maintainers

Thank you for contributing to SplitRS! 🦀✂️
