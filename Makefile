# Job Hunt workspace
#
#   make            list the targets
#   make serve      run the dashboard (client + API) in the foreground
#   make start      start the dashboard as a systemd user service
#
# The server lives behind one entry point: `python3 -m server <command>`. Run the
# interpreter explicitly as /usr/bin/python3 -- a bare `python3` inside a Hermes
# session resolves to a build with no PyYAML and the scan silently finds nothing.

PYTHON  ?= /usr/bin/python3
PORT    ?= 8787
API_PORT ?= 8788
SERVICE ?= job-dashboard.service

.DEFAULT_GOAL := help
.PHONY: help serve api start stop restart status logs health scan test bump clean image scan-docker timer

help: ## List the targets
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) \
		| sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-10s\033[0m %s\n", $$1, $$2}'

## --- run the app -----------------------------------------------------------

serve: ## Run the dashboard (client + API) in the foreground on :8787
	$(PYTHON) -m server --port $(PORT)

api: ## Run the API only, loopback, on :8788 (the production role)
	$(PYTHON) -m server --api-only --port $(API_PORT)

start: ## Start the dashboard systemd user service
	systemctl --user start $(SERVICE)

stop: ## Stop the dashboard service
	systemctl --user stop $(SERVICE)

restart: ## Restart the dashboard service
	systemctl --user restart $(SERVICE)

status: ## Show the dashboard service status
	systemctl --user status $(SERVICE)

logs: ## Follow the dashboard service logs
	journalctl --user -u $(SERVICE) -f

health: ## Check the running API answers on :8788
	@curl -fsS "http://127.0.0.1:$(API_PORT)/api/dashboard" >/dev/null \
		&& echo "API ok on :$(API_PORT)" \
		|| (echo "API not answering on :$(API_PORT)" >&2; exit 1)

## --- workspace tasks -------------------------------------------------------

scan: ## Run one job scan (5 postings per tier)
	$(PYTHON) -m server scan --per-tier 5

test: ## Run the four smoke tests
	$(PYTHON) server/tests/test_boardstate.py
	$(PYTHON) server/tests/test_dashboard.py
	$(PYTHON) server/tests/test_net.py
	$(PYTHON) server/tests/test_retry.py

bump: ## Bump the client cache-buster after editing client/
	$(PYTHON) bin/client_version.py --bump

## --- containers ----------------------------------------------------------

image: ## Build the job-scan container image (only needed when docker/Dockerfile changes)
	docker compose -f docker/compose.yaml build

scan-docker: ## Run one full scan in the container (sweep-if-stale, scan, mark-reported)
	docker compose -f docker/compose.yaml run --rm scan

timer: ## Show the next scheduled scan slots
	systemctl --user list-timers job-scan.timer --no-pager

clean: ## Remove __pycache__ directories
	find . -type d -name __pycache__ -prune -exec rm -rf {} +
