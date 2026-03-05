build:
	RUSTY_V8_MIRROR="https://github.com/supabase/rusty_v8/releases/download" cargo build 2>&1
run:
	RUSTY_V8_MIRROR="https://github.com/supabase/rusty_v8/releases/download" cargo run start 2>&1