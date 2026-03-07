build:
	cargo build 2>&1
run:
	cargo run start 2>&1

watch:
	cargo run watch --path ./examples/hello/hello.ts --inspect 9229

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