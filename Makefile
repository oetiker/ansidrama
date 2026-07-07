# ansidrama Makefile — build, check, package, and regenerate the demo.
# Shared host policy: cap cargo to 4 cores.

export CARGO_BUILD_JOBS := 4

# Locate the (possibly redirected) cargo target dir and the release binary.
TARGET_DIR := $(shell cargo metadata --format-version 1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')
RELEASE_BIN := $(TARGET_DIR)/release/ansidrama

.PHONY: all release fmt lint test check package man demo help

all: check

# --- development ------------------------------------------------------------

fmt:
	cargo fmt

lint:
	cargo clippy --all-targets -- -D warnings

test:
	cargo test

# Format, lint, and test — run before every commit.
check: fmt lint test
	@echo "All checks passed!"

# Release binary (format + lint first).
release: fmt lint
	cargo build --release

# --- packaging (host target) ------------------------------------------------

# Build .deb + .rpm for the host target from the Cargo.toml metadata.
package: release
	cargo deb --no-build
	cargo generate-rpm
	@echo "Packages written under $(TARGET_DIR)/{debian,generate-rpm}/"

# --- documentation ----------------------------------------------------------

# Preview the man page.
man:
	man ./man/ansidrama.1

# --- demo -------------------------------------------------------------------

# Regenerate the committed README demo (docs/demo/ansidrama.webp).
# Runs from demo/ so the inner `cat`/`record`/`ls` see hello.toml, and puts the
# freshly built binary on PATH for the inner `ansidrama record` call.
demo: release
	cd demo && PATH="$(dir $(RELEASE_BIN)):$$PATH" ansidrama record readme.toml
	@echo "Wrote docs/demo/ansidrama.webp"

# --- help -------------------------------------------------------------------

help:
	@echo "ansidrama Makefile"
	@echo ""
	@echo "  make check     Format, lint (clippy -D warnings), and test"
	@echo "  make release   Build the release binary (fmt + lint first)"
	@echo "  make package   Build .deb + .rpm for the host target"
	@echo "  make man       Preview the man page"
	@echo "  make demo      Regenerate docs/demo/ansidrama.webp"
	@echo "  make help      Show this help"
