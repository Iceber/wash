.DEFAULT_GOAL:=help

.PHONY: build build-watch test test-integration test-all clean help

CARGO ?= cargo
DOCKER ?= docker
PYTHON ?= python3

##@ Helpers

help:  ## Display this help
	@awk 'BEGIN {FS = ":.*##"; printf "\nUsage:\n  make \033[36m<target>\033[0m\n"} /^[a-zA-Z_\-.*]+:.*?##/ { printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2 } /^##@/ { printf "\n\033[1m%s\033[0m\n", substr($$0, 5) } ' $(MAKEFILE_LIST)

clean: ## Clean all tests
	@$(CARGO) clean
	wash drain all

deps-check:
	@$(PYTHON) tools/deps_check.py

##@ Building

build: ## Build the project
	@$(CARGO) build

build-watch: ## Continuously build the project
	@$(CARGO) watch -x build

##@ Testing

test: ## Run unit test suite
	@$(CARGO) nextest run $(CARGO_TEST_TARGET) --no-fail-fast --bin wash
	@$(CARGO) nextest run $(CARGO_TEST_TARGET) --no-fail-fast -p wash-lib --features=cli

test-wash-ci:
	@$(CARGO) nextest run --profile ci --workspace --bin wash
	@$(CARGO) nextest run --profile ci -p wash-lib --features=cli

test-watch: ## Run unit tests continously, can optionally specify a target test filter.
	@$(CARGO) watch -- $(CARGO) nextest run $(TARGET)

test-integration: ## Run the entire integration test suite (with docker compose)
	@$(DOCKER) compose -f ./tools/docker-compose.yml up --detach
	@$(CARGO) nextest run --profile integration -E 'kind(test)'
	@$(DOCKER) compose -f ./tools/docker-compose.yml down

test-integration-ci: ## Run the entire integration test suite only
	@$(CARGO) nextest run --profile ci -E 'kind(test)'

test-integration-watch: ## Run integration test suite continuously
	@$(CARGO) watch -- $(MAKE) test-integration

test-unit: ## Run one or more unit tests
	@$(CARGO) nextest run $(CARGO_TEST_TARGET)

test-unit-watch: ## Run tests continuously
	@$(CARGO) watch -- $(MAKE) test-unit

rust-check: ## Run rust checks
	@$(CARGO) fmt --all --check
	@$(CARGO) clippy --all-features --all-targets --workspace

test-all: ## Run all tests
	$(MAKE) test
	$(MAKE) test-integration
	$(MAKE) rust-check
