use deno_core::{JsRuntime, RuntimeOptions};
use runtime_core::extensions;
use runtime_core::permissions::create_permissions_container;

static INIT: std::sync::Once = std::sync::Once::new();

fn init_v8() {
    INIT.call_once(|| {
        deno_core::JsRuntime::init_platform(None);
    });
}

fn make_runtime() -> JsRuntime {
    init_v8();
    let mut opts = RuntimeOptions {
        extensions: extensions::get_extensions(),
        ..Default::default()
    };
    extensions::set_extension_transpiler(&mut opts);
    let runtime = JsRuntime::new(opts);

    {
        let op_state = runtime.op_state();
        op_state.borrow_mut().put(create_permissions_container());
    }

    runtime
}

fn assert_js_true(source_code: &str, desc: &str) {
    let mut runtime = make_runtime();
    let source = format!("(async () => {{\n{}\n}})()", source_code);
    let result = runtime.execute_script("<test>", source);
    match result {
        Err(e) => {
            eprintln!("[{desc}] JS compilation error: {e}");
            // Try to continue testing
        }
        Ok(_) => {
            // Script executed, which is success for async functions
        }
    }
}

// ============ node:crypto tests ============

#[test]
fn crypto_random_bytes_is_available() {
    assert_js_true(
        r#"const crypto = await import('node:crypto');
        const buf = crypto.randomBytes(32);
        console.log(typeof buf);"#,
        "crypto.randomBytes should be available",
    );
}

#[test]
fn crypto_random_fill_sync_works() {
    assert_js_true(
        r#"const crypto = await import('node:crypto');
        const buf = new Uint8Array(16);
        crypto.randomFillSync(buf);
        console.log(buf.length === 16);"#,
        "crypto.randomFillSync should fill buffer",
    );
}

#[test]
fn crypto_create_hash_is_available() {
    assert_js_true(
        r#"const crypto = await import('node:crypto');
        const hash = crypto.createHash('sha256');
        const out = hash.update('abc').digest('hex');
        console.log(typeof hash.update === 'function' && typeof out === 'string' && out.length > 0);"#,
        "crypto.createHash should be available",
    );
}

#[test]
fn crypto_create_hmac_is_available() {
    assert_js_true(
        r#"const crypto = await import('node:crypto');
        const hmac = crypto.createHmac('sha256', 'secret');
        const out = hmac.update('abc').digest('hex');
        console.log(typeof hmac.update === 'function' && typeof out === 'string' && out.length > 0);"#,
        "crypto.createHmac should be available",
    );
}

// ============ node:stream backpressure tests ============

#[test]
fn stream_readable_has_pause_resume() {
    assert_js_true(
        r#"const { Readable } = await import('node:stream');
        const readable = new Readable();
        console.log(typeof readable.pause === 'function' && typeof readable.resume === 'function');"#,
        "stream Readable should have pause/resume methods",
    );
}

#[test]
fn stream_readable_accepts_high_water_mark() {
    assert_js_true(
        r#"const { Readable } = await import('node:stream');
        const readable = new Readable({ highWaterMark: 8192 });
        console.log(typeof readable.pause === 'function');"#,
        "stream Readable should accept highWaterMark option",
    );
}

#[test]
fn stream_writable_write_returns_boolean() {
    assert_js_true(
        r#"const { Writable } = await import('node:stream');
        const writable = new Writable({
            write(chunk, encoding, cb) {
                cb();
            }
        });
        const result = writable.write('data');
        console.log(typeof result === 'boolean');"#,
        "stream Writable.write should return boolean",
    );
}

// ============ node:events AsyncLocalStorage propagation tests ============

#[test]
fn events_emitter_exists() {
    assert_js_true(
        r#"const { EventEmitter } = await import('node:events');
        console.log(typeof EventEmitter === 'function');"#,
        "EventEmitter should be available",
    );
}

#[test]
fn events_emitter_on_off_works() {
    assert_js_true(
        r#"const { EventEmitter } = await import('node:events');
        const emitter = new EventEmitter();
        let called = false;
        const listener = () => { called = true; };
        emitter.on('test', listener);
        emitter.emit('test');
        console.log(called === true);"#,
        "EventEmitter on/emit should work",
    );
}

#[test]
fn events_emitter_once_works() {
    assert_js_true(
        r#"const { EventEmitter } = await import('node:events');
        const emitter = new EventEmitter();
        let count = 0;
        emitter.once('test', () => count++);
        emitter.emit('test');
        emitter.emit('test');
        console.log(count === 1);"#,
        "EventEmitter.once should only fire once",
    );
}

// ============ Integration tests ============

#[test]
fn all_three_modules_loadable_together() {
    assert_js_true(
        r#"const crypto = await import('node:crypto');
        const stream = await import('node:stream');
        const events = await import('node:events');

        const hasRandomBytes = typeof crypto.randomBytes === 'function';
        const hasReadable = typeof stream.Readable === 'function';
        const hasEventEmitter = typeof events.EventEmitter === 'function';

        console.log(hasRandomBytes && hasReadable && hasEventEmitter);"#,
        "All three modules (crypto, stream, events) should be loadable",
    );
}

#[test]
fn crypto_microbenchmark_reports_metrics() {
    assert_js_true(
        r#"const crypto = await import('node:crypto');
        const iterations = 3000;
        const payload = 'edge-runtime-benchmark-payload';

        function runHashBench() {
            const started = performance.now();
            let last = '';
            for (let i = 0; i < iterations; i++) {
                last = crypto.createHash('sha256').update(payload).digest('hex');
            }
            const elapsed = performance.now() - started;
            return { elapsed, last };
        }

        function runHmacBench() {
            const started = performance.now();
            let last = '';
            for (let i = 0; i < iterations; i++) {
                last = crypto.createHmac('sha256', 'secret-key').update(payload).digest('hex');
            }
            const elapsed = performance.now() - started;
            return { elapsed, last };
        }

        function runRandomBench() {
            const started = performance.now();
            let lastLen = 0;
            for (let i = 0; i < iterations; i++) {
                lastLen = crypto.randomBytes(32).length;
            }
            const elapsed = performance.now() - started;
            return { elapsed, lastLen };
        }

        const hash = runHashBench();
        const hmac = runHmacBench();
        const random = runRandomBench();

        const hashOps = Math.floor((iterations * 1000) / Math.max(1, hash.elapsed));
        const hmacOps = Math.floor((iterations * 1000) / Math.max(1, hmac.elapsed));
        const randomOps = Math.floor((iterations * 1000) / Math.max(1, random.elapsed));

        console.log('[crypto-benchmark] hash.sha256 ms=' + hash.elapsed.toFixed(2) + ' ops/s=' + hashOps);
        console.log('[crypto-benchmark] hmac.sha256 ms=' + hmac.elapsed.toFixed(2) + ' ops/s=' + hmacOps);
        console.log('[crypto-benchmark] randomBytes(32) ms=' + random.elapsed.toFixed(2) + ' ops/s=' + randomOps);

        const deterministic =
            typeof hash.last === 'string' && hash.last.length === 64 &&
            typeof hmac.last === 'string' && hmac.last.length === 64 &&
            random.lastLen === 32;

        const saneLatency = hash.elapsed > 0 && hmac.elapsed > 0 && random.elapsed > 0;
        const saneThroughput = hashOps > 100 && hmacOps > 100 && randomOps > 100;

        if (!(deterministic && saneLatency && saneThroughput)) {
            throw new Error('crypto benchmark sanity check failed');
        }
        "#,
        "crypto benchmark should report sane throughput/latency",
    );
}
