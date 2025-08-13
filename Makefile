# Production-grade Rust project management
.PHONY: check lint format test build clean doc doc-open help

# Default target
all: check

# Help target
help:
	@echo "Available targets:"
	@echo "  check    - Run all checks (lint + test + format-check)"
	@echo "  lint     - Run Clippy with strict production settings"
	@echo "  format   - Format code with rustfmt"
	@echo "  test     - Run all tests"
	@echo "  build    - Build in release mode"
	@echo "  clean    - Clean build artifacts"
	@echo "  doc      - Generate documentation"

# Comprehensive check for production
check: lint test format-check
	@echo "✅ All production checks passed!"

# Strict linting for production
lint:
	@echo "🔍 Running production-grade Clippy checks..."
	cargo clippy --lib --tests --all-features -- \
		-D warnings \
		-D clippy::all \
		-D clippy::correctness \
		-D clippy::suspicious \
		-D clippy::complexity \
		-D clippy::perf \
		-D clippy::style \
		-A clippy::missing_docs_in_private_items \
		-A clippy::missing_errors_doc \
		-A clippy::missing_panics_doc \
		-A clippy::module_name_repetitions \
		-A clippy::similar_names \
		-A clippy::too_many_lines \
		-A clippy::multiple_crate_versions \
		-A clippy::wildcard_dependencies \
		-A clippy::must_use_candidate \
		-A clippy::missing_const_for_fn \
		-A clippy::significant_drop_tightening \
		-A clippy::equatable_if_let \
		-A clippy::missing_fields_in_debug \
		-A clippy::unused_async

# Format code
format:
	@echo "🎨 Formatting code..."
	cargo fmt --all

# Check if code is formatted
format-check:
	@echo "📝 Checking code formatting..."
	cargo fmt --all -- --check

# Run tests with coverage information
test:
	@echo "🧪 Running tests..."
	cargo test --lib --all-features
	@echo "📊 Running library tests specifically..."
	cargo test --lib --quiet

# Build in release mode with optimization
build:
	@echo "🏗️  Building release version..."
	cargo build --release --all-features

# Clean build artifacts
clean:
	@echo "🧹 Cleaning build artifacts..."
	cargo clean

# Generate documentation
doc:
	@echo "📚 Generating documentation..."
	cargo doc --all-features --no-deps

# Generate and open documentation  
doc-open:
	@echo "📚 Generating documentation..."
	cargo doc --all-features --no-deps --open

# Security audit
audit:
	@echo "🔒 Running security audit..."
	cargo audit

# Benchmark if available
bench:
	@echo "⚡ Running benchmarks..."
	cargo bench

# Check for unused dependencies
deps-check:
	@echo "📦 Checking dependencies..."
	cargo machete || echo "Install cargo-machete for dependency checking: cargo install cargo-machete"

# Full production pipeline
ci: lint test format-check build doc
	@echo "🚀 Full CI pipeline completed successfully!"