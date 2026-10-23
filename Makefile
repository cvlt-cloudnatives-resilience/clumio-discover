BIN     := clumio-discover
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
TARGET  ?= $(shell rustc -vV | sed -n 's/^host: //p')
PACKAGE := $(BIN)-$(VERSION)-$(TARGET)

.PHONY: build release test lint fmt policy dist clean

build:
	cargo build

release:
	cargo build --release --locked

test:
	cargo test --workspace --locked

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --locked -- -D warnings

fmt:
	cargo fmt --all

policy:
	cargo run --quiet -- policy > policy/clumio-discover-readonly.json

dist: release
	rm -rf dist/$(PACKAGE)
	mkdir -p dist/$(PACKAGE)
	cp target/release/$(BIN) README.md LICENSE config.example.toml policy/clumio-discover-readonly.json dist/$(PACKAGE)/
	tar -C dist -czf dist/$(PACKAGE).tar.gz $(PACKAGE)
	@echo dist/$(PACKAGE).tar.gz

clean:
	cargo clean
	rm -rf dist
