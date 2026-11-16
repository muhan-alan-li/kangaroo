# Job Hunt
#
#   make                 list the targets
#   make run             build and launch the Rust app
#   make check           format, lint and test everything
#
# This branch builds the Rust app: the engine, the menubar server and the GPUI dashboard.
# The Python workspace, the web client, the container and the systemd units are archived
# under `archive/`; they still run production on `main`.
#
# The Rust build needs the full Xcode toolchain for its Metal shader step. `.cargo/config.toml`
# sets DEVELOPER_DIR for you; the Command Line Tools alone are not enough.

CARGO   ?= cargo
JH_DB   ?= data/state/jobhunt.db
FIXTURE ?= fixtures/import_state
APP     := target/debug/jh-app
SERVER  := target/debug/jh-server

.DEFAULT_GOAL := help
.PHONY: help run run-server build app test fmt fmt-check clippy check import db reset-db clean-rust clean

help: ## List the targets
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) \
		| sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'

## --- the Rust app ----------------------------------------------------------

run: build ## Build and launch the desktop app (it starts the server if needed)
	$(APP)

run-server: build ## Run the menu-bar server in the foreground
	$(SERVER)

build: ## Build the menu-bar server and the desktop app
	$(CARGO) build -p jh-server -p jh-app

test: ## Run every Rust test
	$(CARGO) test --all

fmt: ## Format the Rust code
	$(CARGO) fmt --all

fmt-check: ## Fail if the Rust code is not formatted
	$(CARGO) fmt --all -- --check

clippy: ## Lint the Rust code, warnings are errors
	$(CARGO) clippy --all-targets -- -D warnings

check: fmt-check clippy test ## Format, lint and test

import: ## Load the fixture state into the dev database
	$(CARGO) run -q -p jh-cli -- import --from $(FIXTURE) --db $(JH_DB)

db: ## Show what the dev database holds
	@test -f $(JH_DB) && sqlite3 $(JH_DB) \
		"select 'postings', count(*) from postings union all \
		 select 'seen', count(*) from seen union all \
		 select 'scan_runs', count(*) from scan_runs;" \
		|| echo "no database at $(JH_DB) -- run make import"

reset-db: ## Delete the dev database, so the next start creates an empty one
	rm -f $(JH_DB) $(JH_DB)-shm $(JH_DB)-wal

clean-rust: ## Remove the Rust build directory
	$(CARGO) clean

clean: clean-rust
