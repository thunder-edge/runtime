MIN_WATCH_FREE_KB ?= 5242880
RUN_NOFILE ?= 65535

build:
	cargo build 2>&1
run:
	@cur=$$(ulimit -n); \
	if [ "$$cur" -lt "$(RUN_NOFILE)" ]; then \
		ulimit -n $(RUN_NOFILE) >/dev/null 2>&1 || true; \
	fi; \
	eff=$$(ulimit -n); \
	echo "Starting runtime with RLIMIT_NOFILE=$$eff"; \
	EDGE_RUNTIME_POOL_MAX_ISOLATES=64 \
	EDGE_RUNTIME_POOL_GLOBAL_MAX_ISOLATES=128 \
	EDGE_RUNTIME_POOL_MIN_FREE_MEMORY_MIB=0 \
	EDGE_RUNTIME_CONTEXT_POOL_MAX_CONTEXTS=512 \
	EDGE_RUNTIME_MAX_CONTEXTS_PER_ISOLATE=32 \
	EDGE_RUNTIME_MAX_ACTIVE_REQUESTS_PER_CONTEXT=100 \
	EDGE_RUNTIME_POOL_CAPACITY_WAIT_TIMEOUT_MS=500 \
	EDGE_RUNTIME_POOL_CAPACITY_MAX_WAITERS=100000 \
	cargo run start 2>&1

run-profile:
	@cur=$$(ulimit -n); \
	if [ "$$cur" -lt "$(RUN_NOFILE)" ]; then \
		ulimit -n $(RUN_NOFILE) >/dev/null 2>&1 || true; \
	fi; \
	eff=$$(ulimit -n); \
	echo "Starting runtime with RLIMIT_NOFILE=$$eff"; \
	EDGE_RUNTIME_POOL_MAX_ISOLATES=64 \
	EDGE_RUNTIME_POOL_GLOBAL_MAX_ISOLATES=128 \
	EDGE_RUNTIME_POOL_MIN_FREE_MEMORY_MIB=0 \
	EDGE_RUNTIME_CONTEXT_POOL_MAX_CONTEXTS=512 \
	EDGE_RUNTIME_MAX_CONTEXTS_PER_ISOLATE=16 \
	EDGE_RUNTIME_MAX_ACTIVE_REQUESTS_PER_CONTEXT=16 \
	EDGE_RUNTIME_POOL_CAPACITY_WAIT_TIMEOUT_MS=500 \
	EDGE_RUNTIME_POOL_CAPACITY_MAX_WAITERS=100000 \
	cargo run start 2>&1

run-latency:
	@cur=$$(ulimit -n); \
	if [ "$$cur" -lt "$(RUN_NOFILE)" ]; then \
		ulimit -n $(RUN_NOFILE) >/dev/null 2>&1 || true; \
	fi; \
	eff=$$(ulimit -n); \
	echo "Starting runtime (latency profile) RLIMIT_NOFILE=$$eff"; \
	EDGE_RUNTIME_POOL_MIN_ISOLATES=1 \
	EDGE_RUNTIME_POOL_MAX_ISOLATES=64 \
	EDGE_RUNTIME_POOL_GLOBAL_MAX_ISOLATES=128 \
	EDGE_RUNTIME_POOL_MIN_FREE_MEMORY_MIB=0 \
	EDGE_RUNTIME_CONTEXT_POOL_MAX_CONTEXTS=256 \
	EDGE_RUNTIME_MAX_CONTEXTS_PER_ISOLATE=12 \
	EDGE_RUNTIME_MAX_ACTIVE_REQUESTS_PER_CONTEXT=8 \
	EDGE_RUNTIME_POOL_CAPACITY_WAIT_TIMEOUT_MS=100 \
	EDGE_RUNTIME_POOL_CAPACITY_MAX_WAITERS=10000 \
	cargo run start 2>&1

run-throughput:
	@cur=$$(ulimit -n); \
	if [ "$$cur" -lt "$(RUN_NOFILE)" ]; then \
		ulimit -n $(RUN_NOFILE) >/dev/null 2>&1 || true; \
	fi; \
	eff=$$(ulimit -n); \
	echo "Starting runtime (throughput profile) RLIMIT_NOFILE=$$eff"; \
	EDGE_RUNTIME_POOL_MIN_ISOLATES=1 \
	EDGE_RUNTIME_POOL_MAX_ISOLATES=64 \
	EDGE_RUNTIME_POOL_GLOBAL_MAX_ISOLATES=128 \
	EDGE_RUNTIME_POOL_MIN_FREE_MEMORY_MIB=0 \
	EDGE_RUNTIME_CONTEXT_POOL_MAX_CONTEXTS=512 \
	EDGE_RUNTIME_MAX_CONTEXTS_PER_ISOLATE=16 \
	EDGE_RUNTIME_MAX_ACTIVE_REQUESTS_PER_CONTEXT=16 \
	EDGE_RUNTIME_POOL_CAPACITY_WAIT_TIMEOUT_MS=500 \
	EDGE_RUNTIME_POOL_CAPACITY_MAX_WAITERS=100000 \
	cargo run start 2>&1

run-throughput-1k:
	@cur=$$(ulimit -n); \
	if [ "$$cur" -lt "$(RUN_NOFILE)" ]; then \
		ulimit -n $(RUN_NOFILE) >/dev/null 2>&1 || true; \
	fi; \
	eff=$$(ulimit -n); \
	echo "Starting runtime (throughput 1k profile) RLIMIT_NOFILE=$$eff"; \
	EDGE_RUNTIME_POOL_MIN_ISOLATES=1 \
	EDGE_RUNTIME_POOL_MAX_ISOLATES=1024 \
	EDGE_RUNTIME_POOL_GLOBAL_MAX_ISOLATES=1024 \
	EDGE_RUNTIME_POOL_MIN_FREE_MEMORY_MIB=0 \
	EDGE_RUNTIME_CONTEXT_POOL_MAX_CONTEXTS=5024 \
	EDGE_RUNTIME_MAX_CONTEXTS_PER_ISOLATE=12 \
	EDGE_RUNTIME_MAX_ACTIVE_REQUESTS_PER_CONTEXT=6 \
	EDGE_RUNTIME_POOL_CAPACITY_WAIT_TIMEOUT_MS=500 \
	EDGE_RUNTIME_POOL_CAPACITY_MAX_WAITERS=100000 \
	cargo run start 2>&1
check-watch-disk:
	@free_kb=$$(df -Pk . | awk 'NR==2 {print $$4}'); \
	if [ "$${free_kb}" -lt "$(MIN_WATCH_FREE_KB)" ]; then \
		echo "Error: low disk space for watch build ($${free_kb}KB free)."; \
		echo "Hint: run 'cargo clean' or remove old target artifacts before 'make watch'."; \
		exit 1; \
	fi

watch: check-watch-disk
	cargo run watch --path ./examples/hello/hello.ts --format snapshot --inspect 9229

test-js:
	cargo run -- test --path "./tests/js/**/*.ts" --ignore "./tests/js/lib/**" 2>&1

test-rust-fast:
	cargo test-dev

test-rust-full:
	cargo test-full

test:
	cargo test-dev
	cargo run -- test --path "./tests/js/**/*.ts" --ignore "./tests/js/lib/**" 2>&1

test-full:
	cargo test-full
	cargo run -- test --path "./tests/js/**/*.ts" --ignore "./tests/js/lib/**" 2>&1
release:
	cargo build --release 2>&1
	cp target/release/thunder ./thunder
install: release
	cp target/release/thunder /usr/local/bin/thunder