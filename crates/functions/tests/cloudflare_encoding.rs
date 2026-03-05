use deno_core::{JsRuntime, RuntimeOptions};
use runtime_core::extensions;
use runtime_core::permissions::Permissions;

// This module tests Cloudflare Workers Encoding APIs and Node.js Buffer compatibility
// Reference: https://developers.cloudflare.com/workers/runtime-apis/encoding/

static INIT: std::sync::Once = std::sync::Once::new();

fn init_v8() {
    INIT.call_once(|| {
        deno_core::JsRuntime::init_platform(None, false);
    });
}

fn make_runtime() -> JsRuntime {
    init_v8();
    let mut opts = RuntimeOptions {
        extensions: extensions::get_extensions(),
        ..Default::default()
    };
    extensions::set_extension_transpiler(&mut opts);
    let mut runtime = JsRuntime::new(opts);

    {
        let mut op_state = runtime.op_state();
        op_state.borrow_mut().put(Permissions);
    }

    runtime
}

fn assert_js_true(js: &str, desc: &str) {
    // Ensure we run within a Tokio runtime context for deno_core operations
    if tokio::runtime::Handle::try_current().is_err() {
        // Use current_thread runtime to match deno_fetch expectations (EventSource)
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create runtime");
        rt.block_on(async {
            assert_js_true_async(js, desc).await;
        });
    } else {
        // Already in async context, run directly
        let mut runtime = make_runtime();
        let result = runtime.execute_script("<test>", js.to_string());
        match result {
            Err(e) => panic!("[{desc}] JS execution error: {e}"),
            Ok(val) => {
                let scope = &mut runtime.handle_scope();
                let local = deno_core::v8::Local::new(scope, val);
                assert!(local.is_true(), "[{desc}] expected true, got false");
            }
        }
    }
}

async fn assert_js_true_async(js: &str, desc: &str) {
    let mut runtime = make_runtime();
    let result = runtime.execute_script("<test>", js.to_string());
    match result {
        Err(e) => panic!("[{desc}] JS execution error: {e}"),
        Ok(val) => {
            let scope = &mut runtime.handle_scope();
            let local = deno_core::v8::Local::new(scope, val);
            assert!(local.is_true(), "[{desc}] expected true, got false");
        }
    }
}

// ── TextEncoder & TextDecoder ──────────────────────────────────────

#[test]
fn encoding_text_encoder() {
    assert_js_true(
        "(() => {
            const encoder = new TextEncoder();
            const encoded = encoder.encode('Hello, World!');

            return encoded instanceof Uint8Array && encoded.length > 0;
        })()",
        "TextEncoder works",
    );
}

#[test]
fn encoding_text_decoder() {
    assert_js_true(
        "(() => {
            const uint8array = new Uint8Array([72, 101, 108, 108, 111]);
            const decoder = new TextDecoder();
            const decoded = decoder.decode(uint8array);

            return decoded === 'Hello';
        })()",
        "TextDecoder works",
    );
}

#[test]
fn encoding_text_roundtrip() {
    assert_js_true(
        "(() => {
            const original = 'Hello, Cloudflare Workers!';
            const encoder = new TextEncoder();
            const decoder = new TextDecoder();

            const encoded = encoder.encode(original);
            const decoded = decoder.decode(encoded);

            return decoded === original;
        })()",
        "TextEncoder/TextDecoder roundtrip",
    );
}

#[test]
fn encoding_text_streams() {
    assert_js_true(
        "(() => {
            // TextEncoderStream for streaming encoding
            const hasTextEncoderStream = typeof TextEncoderStream === 'function';
            const hasTextDecoderStream = typeof TextDecoderStream === 'function';

            return hasTextEncoderStream && hasTextDecoderStream;
        })()",
        "TextEncoderStream and TextDecoderStream available",
    );
}

// ── Base64 Encoding ────────────────────────────────────────────────

#[test]
fn encoding_atob_btoa() {
    assert_js_true(
        "atob(btoa('hello')) === 'hello'",
        "atob/btoa base64 encoding",
    );
}

#[test]
fn encoding_btoa_encoding() {
    assert_js_true(
        "(() => {
            const encoded = btoa('Hello, World!');
            return encoded === 'SGVsbG8sIFdvcmxkIQ==';
        })()",
        "btoa creates valid base64",
    );
}

#[test]
fn encoding_atob_decoding() {
    assert_js_true(
        "(() => {
            const decoded = atob('SGVsbG8sIFdvcmxkIQ==');
            return decoded === 'Hello, World!';
        })()",
        "atob decodes valid base64",
    );
}

// ── Compression Streams ────────────────────────────────────────────

#[test]
fn encoding_compression_stream() {
    assert_js_true(
        "(() => {
            const hasCompressionStream = typeof CompressionStream === 'function';
            const hasDecompressionStream = typeof DecompressionStream === 'function';

            return hasCompressionStream && hasDecompressionStream;
        })()",
        "CompressionStream and DecompressionStream available",
    );
}

#[test]
fn encoding_gzip_compression() {
    assert_js_true(
        "(() => {
            // Verify compression with gzip format
            const compressor = new CompressionStream('gzip');
            return typeof compressor.writable === 'object' && typeof compressor.readable === 'object';
        })()",
        "Gzip compression available",
    );
}

#[test]
fn encoding_deflate_compression() {
    assert_js_true(
        "(() => {
            // Deflate and deflate-raw formats supported
            const deflateCompressor = new CompressionStream('deflate');
            const deflateRawCompressor = new CompressionStream('deflate-raw');

            return typeof deflateCompressor === 'object' && typeof deflateRawCompressor === 'object';
        })()",
        "Deflate compression formats available",
    );
}

// ── Node.js Buffer Compatibility ────────────────────────────────────

// NOTE: Node.js Buffer is NOT available in current deno-edge-runtime
// Reason: deno_node extension is not loaded
//
// Alternative: Use Uint8Array and related Web APIs instead of Buffer
//
// Buffer operations can be replaced with:
// - Uint8Array - typed array for byte data
// - DataView - read/write at specific byte positions
// - TextEncoder/TextDecoder - string conversions
// - Blob - binary data container

#[test]
fn buffer_alternative_uint8array() {
    assert_js_true(
        "(() => {
            // Buffer.alloc() -> new Uint8Array()
            const buffer = new Uint8Array(10);
            return buffer.length === 10 && buffer instanceof Uint8Array;
        })()",
        "Uint8Array as Buffer alternative",
    );
}

#[test]
fn buffer_alternative_from_string() {
    assert_js_true(
        "(() => {
            // Buffer.from('hello') -> TextEncoder
            const str = 'hello';
            const buffer = new TextEncoder().encode(str);
            return buffer instanceof Uint8Array;
        })()",
        "Buffer.from() alternative with TextEncoder",
    );
}

#[test]
fn buffer_alternative_to_string() {
    assert_js_true(
        "(() => {
            // buffer.toString() -> TextDecoder
            const bytes = new Uint8Array([104, 101, 108, 108, 111]);
            const str = new TextDecoder().decode(bytes);
            return str === 'hello';
        })()",
        "buffer.toString() alternative with TextDecoder",
    );
}

#[test]
fn buffer_alternative_concat() {
    assert_js_true(
        "(() => {
            // Buffer.concat([...]) -> manual concatenation
            const buf1 = new Uint8Array([1, 2, 3]);
            const buf2 = new Uint8Array([4, 5, 6]);

            // Create combined buffer
            const combined = new Uint8Array(buf1.length + buf2.length);
            combined.set(buf1);
            combined.set(buf2, buf1.length);

            return combined.length === 6 && combined[0] === 1 && combined[5] === 6;
        })()",
        "Buffer.concat() alternative",
    );
}

#[test]
fn buffer_alternative_dataview() {
    assert_js_true(
        "(() => {
            // DataView for structured byte access
            const buffer = new ArrayBuffer(4);
            const view = new DataView(buffer);

            view.setInt32(0, 0x12345678);
            const value = view.getInt32(0);

            return value === 0x12345678;
        })()",
        "DataView for binary data operations",
    );
}

// ── Encoding Summary ────────────────────────────────────────────────

// Supported Cloudflare Encoding APIs:
// ✓ TextEncoder/TextDecoder - UTF-8 encoding
// ✓ TextEncoderStream/TextDecoderStream - streaming variants
// ✓ atob/btoa - Base64 encoding
// ✓ CompressionStream/DecompressionStream - gzip, deflate, deflate-raw
// ✓ Uint8Array - binary data container (Buffer alternative)
// ✓ DataView - structured binary access (Buffer alternative)
//
// Recommended patterns:
// - Use TextEncoder instead of Buffer.from(x, 'utf8')
// - Use TextDecoder instead of Buffer.toString()
// - Use Uint8Array instead of Buffer.alloc()
// - Use CompressionStream instead of zlib module
// - Use DataView for binary struct access
