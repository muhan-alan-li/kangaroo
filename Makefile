# Job Hunt
#
#   make                 list the targets
#   make run             build and launch the Rust app
#   make check           format, lint and test everything
#
# Two halves live on this branch. The top half builds the Rust app: the engine, the menubar
# shell and, later, the dashboard. The bottom half is the Python workspace that still runs
# production on `home` until the Rust scan matches it.
#
# The Rust build needs the full Xcode toolchain for its Metal shader step. `.cargo/config.toml`
# sets DEVELOPER_DIR for you; the Command Line Tools alone are not enough.

CARGO   ?= cargo
JH_DB   ?= data/state/jobhunt.db
FIXTURE ?= fixtures/import_state
APP     := target/debug/jh-app

PYTHON   ?= /usr/bin/python3
PORT     ?= 8787
API_PORT ?= 8788
SERVICE  ?= job-dashboard.service

.DEFAULT_GOAL := help
.PHONY: help run build app test fmt fmt-check clippy check import db reset-db clean-rust \
        serve api scan test-py client bump clean image scan-docker timer

help: ## List the targets
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) \
		| sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'

## --- the Rust app ----------------------------------------------------------

run: build ## Build and launch the app
	$(APP)

build: ## Build the app in debug mode
	$(CARGO) build -p jh-app

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

## --- the Python workspace (production until the Rust scan matches it) ------

serve: ## Run the Python dashboard (client + API) in the foreground on :8787
	$(PYTHON) -m server --port $(PORT)

api: ## Run the Python API only, loopback, on :8788
	$(PYTHON) -m server --api-only --port $(API_PORT)

scan: ## Run one Python job scan
	$(PYTHON) -m server scan --per-tier 5

test-py: ## Run the four Python smoke tests
	$(PYTHON) server/tests/test_boardstate.py
	$(PYTHON) server/tests/test_dashboard.py
	$(PYTHON) server/tests/test_net.py
	$(PYTHON) server/tests/test_retry.py

client: ## Build the client bundles (needs node; client/dist/ is committed)
	cd client && npm run build

bump: ## Bump the client cache-buster after editing client/
	$(PYTHON) -m server bump --bump

image: ## Build the job-scan container image
	docker compose -f docker/compose.yaml build

scan-docker: ## Run one full scan in the container
	docker compose -f docker/compose.yaml run --rm scan

timer: ## Show the next scheduled scan slots
	systemctl --user list-timers job-scan.timer --no-pager

clean: ## Remove __pycache__ directories and the Rust build directory
	find . -type d -name __pycache__ -prune -exec rm -rf {} +
	$(CARGO) clean
