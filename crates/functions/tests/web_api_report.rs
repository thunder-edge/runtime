//! Web Standards compatibility report generator.
//!
//! This test boots a JsRuntime with the production extensions and runs
//! a comprehensive set of JS checks. It then generates a markdown report
//! at `WEB_STANDARDS_REPORT.md` in the project root.

use deno_core::{JsRuntime, PollEventLoopOptions, RuntimeOptions};
use runtime_core::extensions;
use runtime_core::permissions::create_permissions_container;
use std::fmt::Write as FmtWrite;

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

    // Add PermissionsContainer to the op_state so that deno_web and other extensions can access it
    {
        let op_state = runtime.op_state();
        op_state.borrow_mut().put(create_permissions_container());
    }

    runtime
}

#[derive(Debug, Clone)]
struct ApiCheck {
    category: &'static str,
    api: &'static str,
    js_check: &'static str,
    /// "full", "partial", or "none"
    status: String,
}

#[derive(Debug, Clone)]
struct NodeCompatCheck {
    api: &'static str,
    profile: &'static str,
    notes: &'static str,
    js_check: &'static str,
    /// "full", "partial", or "none"
    status: String,
}

/// Evaluate a JS expression and return its string result.
fn eval_js(runtime: &mut JsRuntime, js: &str) -> String {
    // Node compat checks rely heavily on dynamic imports; allow extra pump cycles
    // to avoid classifying slow async resolution as "none".
    for _ in 0..24 {
        let status = eval_js_once(runtime, js);
        if status != "pending" {
            return status;
        }
        if !pump_event_loop(runtime) {
            return "none".to_string();
        }
    }
    "none".to_string()
}

fn eval_js_once(runtime: &mut JsRuntime, js: &str) -> String {
    match runtime.execute_script("<check>", js.to_string()) {
        Ok(val) => {
            deno_core::scope!(scope, runtime);
            let local = val.open(scope);
            if let Some(s) = local.to_string(scope) {
                s.to_rust_string_lossy(scope)
            } else {
                "error".to_string()
            }
        }
        Err(_) => "none".to_string(),
    }
}

fn pump_event_loop(runtime: &mut JsRuntime) -> bool {
        let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
        {
                Ok(rt) => rt,
                Err(_) => return false,
        };

        let local = tokio::task::LocalSet::new();
        local
                .block_on(&rt, async {
                        runtime
                                .run_event_loop(PollEventLoopOptions {
                                        wait_for_inspector: false,
                                        pump_v8_message_loop: true,
                                })
                                .await
                })
                .is_ok()
}

fn define_node_compat_checks() -> Vec<NodeCompatCheck> {
        vec![
                NodeCompatCheck {
                        api: "node:process",
                        profile: "Partial",
                                                notes: "Sandboxed process subset with in-memory `env`, virtual cwd (`/bundle`), and stdio compatibility streams.",
                        js_check: r#"(() => {
                                                        return (
                                                            typeof globalThis.process === 'object' &&
                                                            typeof process.nextTick === 'function' &&
                                                            process.env.PATH === undefined &&
                                                            process.cwd() === '/bundle' &&
                                                            typeof process.stdout?.write === 'function' &&
                                                            typeof process.stderr?.write === 'function'
                                                        ) ? 'partial' : 'none';
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:buffer",
                        profile: "Partial",
                        notes: "Common Buffer operations for SSR/tooling (`from`, `alloc`, `concat`, `byteLength`, `toString`).",
                        js_check: r#"(() => {
                            try {
                                const a = Buffer.from('hello');
                                const b = Buffer.concat([a, Buffer.from(' world')]);
                                return (Buffer.isBuffer(a) && b.toString('utf8') === 'hello world') ? 'partial' : 'none';
                            } catch (_e) {
                                return 'none';
                            }
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:crypto",
                        profile: "Partial",
                        notes: "Subset funcional com `randomBytes`/`randomFill` e hashing/HMAC (`createHash`/`createHmac`) sobre WebCrypto + ops nativas.",
                        js_check: r#"(() => {
                            const key = '__edge_node_crypto_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:crypto').then((m) => {
                                    try {
                                        const bytes = m.randomBytes(8);
                                        const hash = m.createHash('sha256').update('abc').digest('hex');
                                        const hmac = m.createHmac('sha256', 'secret').update('abc').digest('hex');
                                        globalThis[key] =
                                            typeof m.randomFillSync === 'function' &&
                                            bytes?.length === 8 &&
                                            typeof hash === 'string' && hash.length > 0 &&
                                            typeof hmac === 'string' && hmac.length > 0
                                            ? 'partial' : 'none';
                                    } catch (_err) {
                                        globalThis[key] = 'none';
                                    }
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:events",
                        profile: "Partial",
                        notes: "EventEmitter-compatible surface for common listener/emit flows.",
                        js_check: r#"(() => {
                            const key = '__edge_node_events_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:events').then(({ EventEmitter }) => {
                                    const em = new EventEmitter();
                                    let count = 0;
                                    em.on('x', () => count++);
                                    em.emit('x');
                                    globalThis[key] = count === 1 ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:util",
                        profile: "Partial",
                        notes: "Utility subset (`format`, `inspect`, `promisify`, `types`, `MIMEType`) used by dependencies.",
                        js_check: r#"(() => {
                            const key = '__edge_node_util_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:util').then((util) => {
                                    const formatted = util.format('x:%d', 2);
                                    const mime = new util.MIMEType('text/plain; charset=utf-8');
                                    globalThis[key] =
                                        typeof util.promisify === 'function' &&
                                        formatted === 'x:2' &&
                                        mime.essence === 'text/plain' &&
                                        mime.params.get('charset') === 'utf-8'
                                        ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:path",
                        profile: "Partial",
                        notes: "Deterministic path helpers for module/tooling compatibility.",
                        js_check: r#"(() => {
                            const key = '__edge_node_path_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:path').then((path) => {
                                    globalThis[key] = path.join('/a', 'b') === '/a/b' ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:stream",
                        profile: "Partial",
                        notes: "Basic stream primitives/pipeline for compatibility paths; not full Node stream semantics.",
                        js_check: r#"(() => {
                            const key = '__edge_node_stream_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:stream').then((stream) => {
                                    globalThis[key] = typeof stream.Readable === 'function' && typeof stream.pipeline === 'function' ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                    api: "node:os",
                    profile: "Partial",
                        notes: "Contract-stable environment info and deterministic errors for unsupported host-affecting calls.",
                        js_check: r#"(() => {
                            const key = '__edge_node_os_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:os').then((os) => {
                                    let throwsDeterministic = false;
                                    try { os.setPriority(0, 0); } catch (_e) { throwsDeterministic = true; }
                                    globalThis[key] = typeof os.platform === 'function' && throwsDeterministic ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:module",
                        profile: "Partial",
                        notes: "`createRequire` and built-in-only `require()` with explicit deterministic policy for unsupported modules.",
                        js_check: r#"(() => {
                            const key = '__edge_node_module_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:module').then((m) => {
                                    const req = m.createRequire('file:///edge-test.js');
                                    const path = req('path');
                                    let unsupported = false;
                                    try { req('definitely_not_real_builtin'); } catch (_e) { unsupported = true; }
                                    globalThis[key] = typeof path.join === 'function' && unsupported ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:fs",
                        profile: "Partial",
                        notes: "VFS-backed module: `/bundle` read-only, `/tmp` writable/ephemeral, `/dev/null` sink. Host filesystem stays inaccessible.",
                        js_check: r#"(() => {
                            const key = '__edge_node_fs_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:fs').then((fs) => {
                                    let writableTmp = false;
                                    let bundleReadOnly = false;
                                    try {
                                        fs.writeFileSync('/tmp/report.txt', 'ok');
                                        writableTmp = fs.readFileSync('/tmp/report.txt', 'utf8') === 'ok';
                                    } catch (err) {
                                        writableTmp = false;
                                    }
                                    try {
                                        fs.writeFileSync('/bundle/blocked.txt', 'x');
                                    } catch (err) {
                                        bundleReadOnly = err?.code === 'EROFS';
                                    }
                                    globalThis[key] = typeof fs.existsSync === 'function' && writableTmp && bundleReadOnly ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:fs/promises",
                        profile: "Partial",
                        notes: "Promise APIs mirror VFS behavior, including writable `/tmp` and deterministic quota/read-only errors.",
                        js_check: r#"(() => {
                            const key = '__edge_node_fs_promises_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:fs/promises').then((fsp) => {
                                    fsp.writeFile('/tmp/report-promises.txt', 'ok')
                                        .then(() => fsp.readFile('/tmp/report-promises.txt', 'utf8'))
                                        .then((value) => { globalThis[key] = value === 'ok' ? 'partial' : 'none'; })
                                        .catch(() => { globalThis[key] = 'none'; });
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:async_hooks",
                        profile: "Partial",
                        notes: "`AsyncLocalStorage` and hook callbacks propagate context across common async boundaries (Promise/microtask).",
                        js_check: r#"(() => {
                            const key = '__edge_node_async_hooks_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:async_hooks').then((m) => {
                                    const als = new m.AsyncLocalStorage();
                                    let promiseCtx;
                                    let microtaskCtx;
                                    const hook = m.createHook({
                                        init: () => {},
                                    }).enable();

                                    als.run('ctx-promise', () => {
                                        Promise.resolve().then(() => {
                                            promiseCtx = als.getStore();
                                        });
                                    });

                                    als.run('ctx-microtask', () => {
                                        queueMicrotask(() => {
                                            microtaskCtx = als.getStore();
                                        });
                                    });

                                    Promise.resolve().then(() => {
                                        hook.disable();
                                        globalThis[key] =
                                            typeof m.executionAsyncId === 'function' &&
                                            typeof m.triggerAsyncId === 'function' &&
                                            promiseCtx === 'ctx-promise' &&
                                            microtaskCtx === 'ctx-microtask'
                                            ? 'partial' : 'none';
                                    });
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                    api: "node:child_process",
                    profile: "Stub",
                        notes: "Non-functional process-spawn APIs with deterministic not-implemented behavior.",
                        js_check: r#"(() => {
                            const key = '__edge_node_child_process_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:child_process').then((m) => {
                                    let deterministic = false;
                                    try { m.exec('echo hi'); } catch (err) { deterministic = String(err?.message || '').includes('not implemented'); }
                                    globalThis[key] = typeof m.spawn === 'function' && deterministic ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                    api: "node:cluster",
                    profile: "Stub",
                        notes: "Non-functional cluster orchestration APIs with deterministic failures.",
                        js_check: r#"(() => {
                            const key = '__edge_node_cluster_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:cluster').then((m) => {
                                    let deterministic = false;
                                    try { m.fork(); } catch (err) { deterministic = String(err?.message || '').includes('not implemented'); }
                                    globalThis[key] = typeof m.isPrimary === 'boolean' && deterministic ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:console",
                        profile: "Partial",
                        notes: "Console module compatibility maps to runtime console implementation.",
                        js_check: r#"(() => {
                            const key = '__edge_node_console_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:console').then((m) => {
                                    globalThis[key] = typeof m.default?.log === 'function' ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:diagnostics_channel",
                        profile: "Partial",
                        notes: "Basic publish/subscribe channel plus `TracingChannel` hooks for sync/promise tracing flows.",
                        js_check: r#"(() => {
                            const key = '__edge_node_diag_channel_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:diagnostics_channel').then((m) => {
                                    const ch = m.channel('edge');
                                    let called = false;
                                    const fn = () => { called = true; };
                                    ch.subscribe(fn);
                                    ch.publish({ ok: true });
                                    ch.unsubscribe(fn);

                                    const tracer = new m.TracingChannel('edge.trace');
                                    let starts = 0;
                                    let ends = 0;
                                    tracer.subscribe({
                                        start: () => { starts++; },
                                        end: () => { ends++; },
                                    });
                                    const result = tracer.traceSync((a, b) => a + b, null, 2, 3);

                                    globalThis[key] = called && starts === 1 && ends === 1 && result === 5 ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:dns",
                        profile: "Partial",
                        notes: "DoH-backed subset (`lookup`, `resolve*`, `reverse`) with bounded answers/timeouts; unsupported APIs remain deterministic stubs.",
                        js_check: r#"(() => {
                            const key = '__edge_node_dns_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:dns').then((m) => {
                                    const prev = globalThis.__edgeMockFetchHandler;
                                    globalThis.__edgeMockFetchHandler = async (input) => {
                                        const raw = typeof input === 'string' ? input : input?.url;
                                        const url = new URL(String(raw || 'https://example.com/'));
                                        if (url.pathname.includes('dns-query')) {
                                            const type = (url.searchParams.get('type') || 'A').toUpperCase();
                                            const name = (url.searchParams.get('name') || '').toLowerCase();
                                            if (type === 'A' && name === 'example.com') {
                                                return new Response(JSON.stringify({
                                                    Status: 0,
                                                    Answer: [{ type: 1, data: '93.184.216.34' }],
                                                }), { status: 200, headers: { 'content-type': 'application/dns-json' } });
                                            }
                                            if (type === 'AAAA' && name === 'example.com') {
                                                return new Response(JSON.stringify({ Status: 0, Answer: [] }), {
                                                    status: 200,
                                                    headers: { 'content-type': 'application/dns-json' },
                                                });
                                            }
                                            if (type === 'PTR' && name === '34.216.184.93.in-addr.arpa') {
                                                return new Response(JSON.stringify({
                                                    Status: 0,
                                                    Answer: [{ type: 12, data: 'example.com' }],
                                                }), { status: 200, headers: { 'content-type': 'application/dns-json' } });
                                            }
                                            return new Response(JSON.stringify({ Status: 3, Answer: [] }), {
                                                status: 200,
                                                headers: { 'content-type': 'application/dns-json' },
                                            });
                                        }
                                        return new Response('ok', { status: 200 });
                                    };

                                    m.lookup('example.com', (err, address, family) => {
                                        if (err || address !== '93.184.216.34' || family !== 4) {
                                            globalThis.__edgeMockFetchHandler = prev;
                                            globalThis[key] = 'none';
                                            return;
                                        }
                                        m.resolve4('example.com', (resolveErr, records) => {
                                            if (resolveErr || !Array.isArray(records) || records[0] !== '93.184.216.34') {
                                                globalThis.__edgeMockFetchHandler = prev;
                                                globalThis[key] = 'none';
                                                return;
                                            }
                                            m.reverse('93.184.216.34', (reverseErr, hostnames) => {
                                                globalThis.__edgeMockFetchHandler = prev;
                                                globalThis[key] = (!reverseErr && Array.isArray(hostnames) && hostnames[0] === 'example.com') ? 'partial' : 'none';
                                            });
                                        });
                                    });
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                    api: "node:http",
                    profile: "Partial",
                                                notes: "HTTP client compatibility is provided as a wrapper around `fetch()`; `createServer` is an importable limited stub and `Server.listen` fails deterministically.",
                        js_check: r#"(() => {
                            const key = '__edge_node_http_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:http').then((m) => {
                                                                        let serverStubDeterministic = false;
                                                                        try {
                                                                            m.createServer(() => {}).listen(8080);
                                                                        } catch (err) {
                                                                            serverStubDeterministic = String(err?.message || '').includes('[thunder] http.Server.listen is not implemented in this runtime profile');
                                                                        }
                                                                        const prev = globalThis.__edgeMockFetchHandler;
                                                                        globalThis.__edgeMockFetchHandler = async () => new Response('http-ok', { status: 200 });
                                                                        m.get('https://example.com', (res) => {
                                                                            let body = '';
                                                                            res.on('data', (chunk) => { body += String(chunk); });
                                                                            res.on('end', () => {
                                                                                globalThis.__edgeMockFetchHandler = prev;
                                                                                globalThis[key] = Array.isArray(m.METHODS) && typeof m.request === 'function' && typeof m.createServer === 'function' && serverStubDeterministic && res.statusCode === 200 && body.includes('http-ok') ? 'partial' : 'none';
                                                                            });
                                                                            res.on('error', () => {
                                                                                globalThis.__edgeMockFetchHandler = prev;
                                                                                globalThis[key] = 'none';
                                                                            });
                                                                        }).on('error', () => {
                                                                            globalThis.__edgeMockFetchHandler = prev;
                                                                            globalThis[key] = 'none';
                                                                        });
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                    api: "node:https",
                    profile: "Partial",
                                                notes: "HTTPS client compatibility is provided as a wrapper around `fetch()`; server-side APIs remain non-functional.",
                        js_check: r#"(() => {
                            const key = '__edge_node_https_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:https').then((m) => {
                                                                        const prev = globalThis.__edgeMockFetchHandler;
                                                                        globalThis.__edgeMockFetchHandler = async () => new Response('https-ok', { status: 200 });
                                                                        m.request('https://example.com', { method: 'GET' }, (res) => {
                                                                            let body = '';
                                                                            res.on('data', (chunk) => { body += String(chunk); });
                                                                            res.on('end', () => {
                                                                                globalThis.__edgeMockFetchHandler = prev;
                                                                                globalThis[key] = typeof m.request === 'function' && typeof m.get === 'function' && res.statusCode === 200 && body.includes('https-ok') ? 'partial' : 'none';
                                                                            });
                                                                            res.on('error', () => {
                                                                                globalThis.__edgeMockFetchHandler = prev;
                                                                                globalThis[key] = 'none';
                                                                            });
                                                                        }).on('error', () => {
                                                                            globalThis.__edgeMockFetchHandler = prev;
                                                                            globalThis[key] = 'none';
                                                                        }).end();
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                    api: "node:http2",
                    profile: "Stub",
                        notes: "HTTP/2 compatibility surface for imports with deterministic non-functional operations.",
                        js_check: r#"(() => {
                            const key = '__edge_node_http2_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:http2').then((m) => {
                                    globalThis[key] = typeof m.createServer === 'function' && typeof m.constants === 'object' ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                    api: "node:inspector",
                    profile: "Stub",
                        notes: "Inspector bridge compatibility surface with no-op/open stubs in this runtime profile.",
                        js_check: r#"(() => {
                            const key = '__edge_node_inspector_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:inspector').then((m) => {
                                    globalThis[key] = typeof m.open === 'function' && typeof m.Session === 'function' ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:net",
                        profile: "Partial",
                        notes: "Outbound client socket subset (`connect`) is available; `net.Server` APIs remain deterministic stubs.",
                        js_check: r#"(() => {
                            const key = '__edge_node_net_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:net').then((m) => {
                                    let deterministic = false;
                                    try { m.createServer().listen(80); } catch (err) { deterministic = String(err?.message || '').includes('not implemented'); }
                                    globalThis[key] = typeof m.connect === 'function' && typeof m.Socket === 'function' && deterministic ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:perf_hooks",
                        profile: "Partial",
                        notes: "Performance hooks compatibility based on runtime Performance APIs.",
                        js_check: r#"(() => {
                            const key = '__edge_node_perf_hooks_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:perf_hooks').then((m) => {
                                    globalThis[key] = (typeof m.performance?.now === 'function' || typeof globalThis.performance?.now === 'function') ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:punycode",
                        profile: "Partial",
                        notes: "Punycode compatibility helpers for import-level ecosystem support.",
                        js_check: r#"(() => {
                            const key = '__edge_node_punycode_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:punycode').then((m) => {
                                    const out = m.toASCII('example.com');
                                    globalThis[key] = typeof out === 'string' ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:querystring",
                        profile: "Partial",
                        notes: "Querystring parse/stringify compatibility helpers.",
                        js_check: r#"(() => {
                            const key = '__edge_node_querystring_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:querystring').then((m) => {
                                    const parsed = m.parse('a=1&b=2');
                                    globalThis[key] = parsed.a === '1' && m.stringify({ a: 1 }) === 'a=1' ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                    api: "node:readline",
                    profile: "Stub",
                        notes: "Readline import-level compatibility with deterministic non-functional interactive APIs.",
                        js_check: r#"(() => {
                            const key = '__edge_node_readline_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:readline').then((m) => {
                                    const i = m.createInterface({});
                                    let deterministic = false;
                                    try { i.question('x'); } catch (err) { deterministic = String(err?.message || '').includes('not implemented'); }
                                    globalThis[key] = deterministic ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                    api: "node:repl",
                    profile: "Stub",
                        notes: "REPL compatibility entrypoint with deterministic non-functional behavior.",
                        js_check: r#"(() => {
                            const key = '__edge_node_repl_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:repl').then((m) => {
                                    let deterministic = false;
                                    try { m.start(); } catch (err) { deterministic = String(err?.message || '').includes('not implemented'); }
                                    globalThis[key] = deterministic ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:sqlite",
                        profile: "Stub",
                        notes: "Import-compatible stub module. Constructors fail deterministically when invoked.",
                        js_check: r#"(() => {
                            const key = '__edge_node_sqlite_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:sqlite').then((m) => {
                                    let deterministic = false;
                                    try { new m.Database(':memory:'); } catch (err) { deterministic = String(err?.message || '').includes('is not implemented'); }
                                    globalThis[key] = deterministic ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:string_decoder",
                        profile: "Partial",
                        notes: "String decoder compatibility for common buffer decoding flows.",
                        js_check: r#"(() => {
                            const key = '__edge_node_string_decoder_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:string_decoder').then((m) => {
                                    const d = new m.StringDecoder('utf-8');
                                    globalThis[key] = d.end(new Uint8Array([65])) === 'A' ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:test",
                        profile: "Stub",
                        notes: "Import-compatible test module stub. Methods throw deterministic not-implemented errors.",
                        js_check: r#"(() => {
                            const key = '__edge_node_test_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:test').then((m) => {
                                    let deterministic = false;
                                    try { m.test('x', () => {}); } catch (err) { deterministic = String(err?.message || '').includes('is not implemented'); }
                                    globalThis[key] = deterministic ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:tls",
                        profile: "Partial",
                        notes: "Outbound TLS client subset (`connect`) is available; server/context APIs remain deterministic stubs.",
                        js_check: r#"(() => {
                            const key = '__edge_node_tls_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:tls').then((m) => {
                                    let deterministic = false;
                                    try { m.createServer(); } catch (err) { deterministic = String(err?.message || '').includes('not implemented'); }
                                    globalThis[key] = Array.isArray(m.rootCertificates) && typeof m.connect === 'function' && deterministic ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                    api: "node:dgram",
                    profile: "Stub",
                        notes: "UDP/datagram import compatibility with deterministic non-functional sockets.",
                        js_check: r#"(() => {
                            const key = '__edge_node_dgram_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:dgram').then((m) => {
                                    let deterministic = false;
                                    try { m.createSocket('udp4'); } catch (err) { deterministic = String(err?.message || '').includes('not implemented'); }
                                    globalThis[key] = deterministic ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                    api: "node:v8",
                    profile: "Partial",
                        notes: "V8 compatibility introspection helpers with deterministic static values.",
                        js_check: r#"(() => {
                            const key = '__edge_node_v8_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:v8').then((m) => {
                                    globalThis[key] = typeof m.cachedDataVersionTag === 'function' && typeof m.getHeapStatistics === 'function' ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                    api: "node:vm",
                    profile: "Stub",
                        notes: "VM import compatibility with deterministic non-functional script execution APIs.",
                        js_check: r#"(() => {
                            const key = '__edge_node_vm_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:vm').then((m) => {
                                    let deterministic = false;
                                    try { new m.Script('1+1').runInThisContext(); } catch (err) { deterministic = String(err?.message || '').includes('not implemented'); }
                                    globalThis[key] = deterministic ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                    api: "node:worker_threads",
                    profile: "Stub",
                    notes: "Worker threads module is importable for feature detection; thread-spawning APIs fail deterministically by sandbox policy.",
                    js_check: r#"(() => {
                        const key = '__edge_node_worker_threads_check';
                        if (globalThis[key] === undefined) {
                            globalThis[key] = 'pending';
                            import('node:worker_threads').then((m) => {
                                let deterministic = false;
                                try { new m.Worker('file:///tmp/test.js'); } catch (err) { deterministic = String(err?.message || '').includes('not implemented'); }
                                globalThis[key] = m.isMainThread === true && deterministic ? 'partial' : 'none';
                            }).catch(() => {
                                globalThis[key] = 'none';
                            });
                            return 'pending';
                        }
                        return globalThis[key];
                    })()"#,
                    status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:zlib",
                        profile: "Partial",
                        notes: "Functional async+sync one-shot compression subset (`gzip/gunzip/deflate/inflate/deflateRaw/inflateRaw`) backed by native runtime ops with runtime-configurable defaults under immutable hard output/input ceilings and operation-time guardrail; stream constructors remain deterministic stubs.",
                        js_check: r#"(() => {
                            const key = '__edge_node_zlib_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:zlib').then((m) => {
                                    m.gzip('report-zlib', (gzipErr, gz) => {
                                        if (gzipErr || !gz) {
                                            globalThis[key] = 'none';
                                            return;
                                        }
                                        m.gunzip(gz, (gunzipErr, plain) => {
                                            if (gunzipErr || !plain) {
                                                globalThis[key] = 'none';
                                                return;
                                            }
                                            const text = typeof plain === 'string' ? plain : new TextDecoder().decode(plain);
                                            let syncCompat = false;
                                            try {
                                                const syncGz = m.gzipSync('report-zlib-sync');
                                                const syncPlain = m.gunzipSync(syncGz);
                                                const syncText = typeof syncPlain === 'string' ? syncPlain : new TextDecoder().decode(syncPlain);
                                                syncCompat = syncText === 'report-zlib-sync';
                                            } catch (_) {
                                                syncCompat = false;
                                            }
                                            globalThis[key] =
                                                text === 'report-zlib' &&
                                                typeof m.deflate === 'function' &&
                                                typeof m.inflate === 'function' &&
                                                typeof m.deflateRaw === 'function' &&
                                                typeof m.inflateRaw === 'function' &&
                                                syncCompat
                                                ? 'partial' : 'none';
                                        });
                                    });
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:assert",
                        profile: "Partial",
                        notes: "Assertion testing helpers compatible with common assert usage patterns.",
                        js_check: r#"(() => {
                            const key = '__edge_node_assert_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:assert').then((m) => {
                                    let threw = false;
                                    try { m.strictEqual(1, 2); } catch (_e) { threw = true; }
                                    globalThis[key] = typeof m.ok === 'function' && threw ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:url",
                        profile: "Partial",
                        notes: "URL module compatibility with URL constructors, file URL helpers, and domain ASCII/Unicode helpers.",
                        js_check: r#"(() => {
                            const key = '__edge_node_url_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:url').then((m) => {
                                    const p = m.fileURLToPath('file:///tmp/a.txt');
                                    const u = m.pathToFileURL('/tmp/a.txt');
                                    const a = m.domainToASCII('español.com');
                                    const unicode = m.domainToUnicode('xn--espaol-zwa.com');
                                    globalThis[key] = typeof m.URL === 'function' && p.endsWith('/tmp/a.txt') && u.protocol === 'file:' && typeof a === 'string' && typeof unicode === 'string' ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:timers",
                        profile: "Partial",
                        notes: "Timer module compatibility backed by runtime timer globals.",
                        js_check: r#"(() => {
                            const key = '__edge_node_timers_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:timers').then((m) => {
                                    globalThis[key] = typeof m.setTimeout === 'function' && typeof m.clearTimeout === 'function' ? 'partial' : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
                NodeCompatCheck {
                        api: "node:timers/promises",
                        profile: "Partial",
                        notes: "Promise-based timers compatibility (`setTimeout`, `setImmediate`, `setInterval`).",
                        js_check: r#"(() => {
                            const key = '__edge_node_timers_promises_check';
                            if (globalThis[key] === undefined) {
                                globalThis[key] = 'pending';
                                import('node:timers/promises').then((m) => {
                                    globalThis[key] =
                                      typeof m.setTimeout === 'function' &&
                                      typeof m.setImmediate === 'function' &&
                                      typeof m.setInterval === 'function'
                                        ? 'partial'
                                        : 'none';
                                }).catch(() => {
                                    globalThis[key] = 'none';
                                });
                                return 'pending';
                            }
                            return globalThis[key];
                        })()"#,
                        status: String::new(),
                },
        ]
}

fn status_label(status: &str) -> &'static str {
        match status {
                "full" => "Full",
                "partial" => "Partial",
                _ => "None",
        }
}

fn node_support_tier(profile: &str, _status: &str) -> &'static str {
    match profile {
        "Full" => "Full",
        "Stub" => "Stub",
        _ => "Partial",
    }
}

fn define_checks() -> Vec<ApiCheck> {
    // Each check returns a JS expression that evaluates to "full", "partial", or "none".
    vec![
        // ── Fetch API ──
        ApiCheck {
            category: "Fetch API",
            api: "Headers",
            js_check: r#"(() => { try { const h = new Headers({'x': 'y'}); return (h.get('x') === 'y' && typeof h.append === 'function' && typeof h.delete === 'function' && typeof h.entries === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Fetch API",
            api: "Request",
            js_check: r#"(() => { try { const r = new Request('http://example.com', {method: 'POST', headers: {'x':'y'}}); return (r.method === 'POST' && r.headers.get('x') === 'y' && typeof r.json === 'function' && typeof r.text === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Fetch API",
            api: "Response",
            js_check: r#"(() => { try { const r = new Response('ok', {status: 201, headers: {'x':'y'}}); return (r.status === 201 && r.headers.get('x') === 'y' && typeof r.json === 'function' && typeof r.text === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Fetch API",
            api: "fetch()",
            js_check: r#"(() => { return typeof fetch === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Fetch API",
            api: "FormData",
            js_check: r#"(() => { try { const fd = new FormData(); fd.append('k','v'); return (fd.get('k') === 'v' && typeof fd.entries === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Fetch API",
            api: "EventSource",
            js_check: r#"(() => { return typeof EventSource === 'function' ? 'partial' : 'none'; })()"#,
            status: String::new(),
        },
        // ── URL API ──
        ApiCheck {
            category: "URL API",
            api: "URL",
            js_check: r#"(() => { try { const u = new URL('https://example.com:8080/path?q=1#h'); return (u.hostname === 'example.com' && u.port === '8080' && u.pathname === '/path' && u.search === '?q=1' && u.hash === '#h') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "URL API",
            api: "URLSearchParams",
            js_check: r#"(() => { try { const p = new URLSearchParams('a=1&b=2&a=3'); return (p.get('b') === '2' && p.getAll('a').length === 2 && typeof p.entries === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "URL API",
            api: "URLPattern",
            js_check: r#"(() => { try { const p = new URLPattern({pathname: '/api/:id'}); const result = p.exec('http://example.com/api/123'); return (result && result.pathname.groups.id === '123') ? 'full' : 'partial'; } catch(e) { return typeof URLPattern === 'function' ? 'partial' : 'none'; } })()"#,
            status: String::new(),
        },
        // ── Streams API ──
        ApiCheck {
            category: "Streams API",
            api: "ReadableStream",
            js_check: r#"(() => { try { const rs = new ReadableStream({ start(c) { c.enqueue('data'); c.close(); } }); return (rs instanceof ReadableStream && typeof rs.getReader === 'function' && typeof rs.pipeTo === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Streams API",
            api: "WritableStream",
            js_check: r#"(() => { try { const ws = new WritableStream(); return (ws instanceof WritableStream && typeof ws.getWriter === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Streams API",
            api: "TransformStream",
            js_check: r#"(() => { try { const ts = new TransformStream(); return (ts.readable instanceof ReadableStream && ts.writable instanceof WritableStream) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Streams API",
            api: "ByteLengthQueuingStrategy",
            js_check: r#"(() => { try { new ByteLengthQueuingStrategy({ highWaterMark: 1024 }); return 'full'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Streams API",
            api: "CountQueuingStrategy",
            js_check: r#"(() => { try { new CountQueuingStrategy({ highWaterMark: 10 }); return 'full'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        // ── Encoding API ──
        ApiCheck {
            category: "Encoding API",
            api: "TextEncoder",
            js_check: r#"(() => { try { const e = new TextEncoder(); const arr = e.encode('hello'); return (arr.length === 5 && arr instanceof Uint8Array) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Encoding API",
            api: "TextDecoder",
            js_check: r#"(() => { try { const d = new TextDecoder('utf-8'); const result = d.decode(new Uint8Array([72,101,108,108,111])); return result === 'Hello' ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Encoding API",
            api: "TextEncoderStream",
            js_check: r#"(() => { try { const s = new TextEncoderStream(); return (s.readable instanceof ReadableStream && s.writable instanceof WritableStream) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Encoding API",
            api: "TextDecoderStream",
            js_check: r#"(() => { try { const s = new TextDecoderStream(); return (s.readable instanceof ReadableStream && s.writable instanceof WritableStream) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Encoding API",
            api: "atob / btoa",
            js_check: r#"(() => { try { return (atob(btoa('test')) === 'test') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        // ── Crypto API ──
        ApiCheck {
            category: "Crypto API",
            api: "crypto.getRandomValues()",
            js_check: r#"(() => { try { const arr = new Uint8Array(16); crypto.getRandomValues(arr); return arr.some(v => v !== 0) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Crypto API",
            api: "crypto.randomUUID()",
            js_check: r#"(() => { try { const uuid = crypto.randomUUID(); return (typeof uuid === 'string' && uuid.length === 36 && uuid.split('-').length === 5) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Crypto API",
            api: "crypto.subtle",
            js_check: r#"(() => { try { return (typeof crypto.subtle === 'object' && typeof crypto.subtle.digest === 'function' && typeof crypto.subtle.generateKey === 'function' && typeof crypto.subtle.sign === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Crypto API",
            api: "CryptoKey",
            js_check: r#"(() => { return typeof CryptoKey === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        // ── Console API ──
        ApiCheck {
            category: "Console API",
            api: "console.log / error / warn",
            js_check: r#"(() => { return (typeof console === 'object' && typeof console.log === 'function' && typeof console.error === 'function' && typeof console.warn === 'function' && typeof console.info === 'function') ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Console API",
            api: "console.table / trace / dir",
            js_check: r#"(() => { return (typeof console.table === 'function' && typeof console.trace === 'function' && typeof console.dir === 'function') ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        // ── Timers API ──
        ApiCheck {
            category: "Timers API",
            api: "setTimeout / clearTimeout",
            js_check: r#"(() => { return (typeof setTimeout === 'function' && typeof clearTimeout === 'function') ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Timers API",
            api: "setInterval / clearInterval",
            js_check: r#"(() => { return (typeof setInterval === 'function' && typeof clearInterval === 'function') ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        // ── Events API ──
        ApiCheck {
            category: "Events API",
            api: "Event",
            js_check: r#"(() => { try { const e = new Event('test', {bubbles: true}); return (e.type === 'test' && e.bubbles === true) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Events API",
            api: "EventTarget",
            js_check: r#"(() => { try { const et = new EventTarget(); let fired = false; et.addEventListener('x', () => { fired = true; }); et.dispatchEvent(new Event('x')); return fired ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Events API",
            api: "CustomEvent",
            js_check: r#"(() => { try { const e = new CustomEvent('foo', { detail: { key: 42 } }); return (e.type === 'foo' && e.detail.key === 42) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Events API",
            api: "AbortController / AbortSignal",
            js_check: r#"(() => { try { const ac = new AbortController(); ac.abort('reason'); return (ac.signal.aborted === true && typeof ac.signal.addEventListener === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Events API",
            api: "ErrorEvent",
            js_check: r#"(() => { try { const e = new ErrorEvent('error', {message: 'oops'}); return e.message === 'oops' ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Events API",
            api: "PromiseRejectionEvent",
            js_check: r#"(() => { try { return typeof PromiseRejectionEvent === 'function' ? 'full' : 'none'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        // ── DOM API ──
        ApiCheck {
            category: "DOM API",
            api: "DOMException",
            js_check: r#"(() => { try { const e = new DOMException('msg', 'NotFoundError'); return (e.name === 'NotFoundError' && e.message === 'msg' && e.code === 8) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "DOM API",
            api: "structuredClone",
            js_check: r#"(() => { try { const obj = {a: 1, b: [2,3], c: new Date()}; const clone = structuredClone(obj); return (clone.a === 1 && clone.b[1] === 3 && clone !== obj && clone.c instanceof Date) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        // ── File API ──
        ApiCheck {
            category: "File API",
            api: "Blob",
            js_check: r#"(() => { try { const b = new Blob(['hello', ' world'], {type: 'text/plain'}); return (b.size === 11 && b.type === 'text/plain' && typeof b.text === 'function' && typeof b.arrayBuffer === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "File API",
            api: "File",
            js_check: r#"(() => { try { const f = new File(['data'], 'test.txt', {type: 'text/plain'}); return (f.name === 'test.txt' && f.size === 4 && f.type === 'text/plain') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "File API",
            api: "FileReader",
            js_check: r#"(() => { return typeof FileReader === 'function' ? 'partial' : 'none'; })()"#,
            status: String::new(),
        },
        // ── Compression API ──
        ApiCheck {
            category: "Compression API",
            api: "CompressionStream",
            js_check: r#"(() => { try { const cs = new CompressionStream('gzip'); return (cs.readable instanceof ReadableStream && cs.writable instanceof WritableStream) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Compression API",
            api: "DecompressionStream",
            js_check: r#"(() => { try { const ds = new DecompressionStream('gzip'); return (ds.readable instanceof ReadableStream && ds.writable instanceof WritableStream) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        // ── Performance API ──
        ApiCheck {
            category: "Performance API",
            api: "performance.now()",
            js_check: r#"(() => { try { const t = performance.now(); return (typeof t === 'number' && t >= 0) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Performance API",
            api: "PerformanceMark / measure",
            js_check: r#"(() => { try { return (typeof PerformanceMark === 'function' && typeof PerformanceMeasure === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        // ── Messaging API ──
        ApiCheck {
            category: "Messaging API",
            api: "MessageChannel / MessagePort",
            js_check: r#"(() => { try { const ch = new MessageChannel(); return (ch.port1 instanceof MessagePort && ch.port2 instanceof MessagePort && typeof ch.port1.postMessage === 'function') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Messaging API",
            api: "ImageData",
            js_check: r#"(() => { try { const id = new ImageData(2, 2); return (id.width === 2 && id.height === 2 && id.data.length === 16) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        // ── HTMLRewriter API ──
        ApiCheck {
            category: "HTML Rewriter",
            api: "HTMLRewriter",
            js_check: r#"(() => { return typeof HTMLRewriter === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        // ── Non-Standard APIs ──
        ApiCheck {
            category: "Non-Standard APIs",
            api: "ScheduledEvent",
            js_check: r#"(() => { return typeof ScheduledEvent === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Non-Standard APIs",
            api: "KV",
            js_check: r#"(() => { return typeof KVNamespace === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Non-Standard APIs",
            api: "Durable Objects",
            js_check: r#"(() => { return (typeof DurableObject === 'function' || typeof DurableObjectNamespace === 'function') ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Non-Standard APIs",
            api: "crypto.DigestStream",
            js_check: r#"(() => { return typeof DigestStream === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Non-Standard APIs",
            api: "Ed25519 via WebCrypto",
            js_check: r#"(() => { try { return (typeof crypto === 'object' && crypto.subtle && typeof crypto.subtle.generateKey === 'function' && typeof crypto.subtle.sign === 'function' && typeof crypto.subtle.verify === 'function') ? 'partial' : 'none'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        // ── General Capabilities ──
        ApiCheck {
            category: "General Capabilities",
            api: "File system access",
            js_check: r#"(() => { return (typeof Deno === 'object' && typeof Deno.readFile === 'function') ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "General Capabilities",
            api: "Connect TCP",
            js_check: r#"(() => { return (typeof Deno === 'object' && typeof Deno.connect === 'function') ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "General Capabilities",
            api: "Connect UDP",
            js_check: r#"(() => { return (typeof Deno === 'object' && typeof Deno.listenDatagram === 'function') ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "General Capabilities",
            api: "WebSockets (Server)",
            js_check: r#"(() => { return (typeof Deno === 'object' && typeof Deno.upgradeWebSocket === 'function') ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        // ── Typed Arrays ──
        ApiCheck {
            category: "Typed Arrays",
            api: "Uint8Array",
            js_check: r#"(() => { try { return (new Uint8Array(8).length === 8) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Typed Arrays",
            api: "Int32Array / Float64Array",
            js_check: r#"(() => { try { return (new Int32Array(4).length === 4 && new Float64Array(2).length === 2) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Typed Arrays",
            api: "ArrayBuffer / DataView",
            js_check: r#"(() => { try { return (new ArrayBuffer(16).byteLength === 16 && new DataView(new ArrayBuffer(8)).byteLength === 8) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        // ── JSON API ──
        ApiCheck {
            category: "JSON API",
            api: "JSON stringify / parse",
            js_check: r#"(() => { try { return (JSON.parse(JSON.stringify({a: 1})).a === 1) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        // ── Promise API ──
        ApiCheck {
            category: "Promise API",
            api: "Promise constructor",
            js_check: r#"(() => { try { return (new Promise(r => r(42)) instanceof Promise) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        // ── Collections ──
        ApiCheck {
            category: "Collections",
            api: "Map / Set",
            js_check: r#"(() => { try { const m = new Map([['a', 1]]); const s = new Set([1, 2, 2]); return (m.get('a') === 1 && s.size === 2) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Collections",
            api: "WeakMap / WeakSet",
            js_check: r#"(() => { return (typeof WeakMap === 'function' && typeof WeakSet === 'function') ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        // ── Symbol API ──
        ApiCheck {
            category: "Symbol API",
            api: "Symbol",
            js_check: r#"(() => { return (typeof Symbol === 'function' && typeof Symbol('test') === 'symbol') ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        // ── Proxy & Reflect ──
        ApiCheck {
            category: "Proxy & Reflect",
            api: "Proxy",
            js_check: r#"(() => { try { return (typeof Proxy === 'function' && new Proxy({}, {}) !== undefined) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Proxy & Reflect",
            api: "Reflect API",
            js_check: r#"(() => { return (typeof Reflect === 'object' && typeof Reflect.get === 'function') ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        // ── Generator & Async ──
        ApiCheck {
            category: "Generators & Async",
            api: "Generator function",
            js_check: r#"(() => { try { function* gen() { yield 1; } return (typeof gen() === 'object') ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Generators & Async",
            api: "Async function",
            js_check: r#"(() => { try { async function test() { return 42; } return (test() instanceof Promise) ? 'full' : 'partial'; } catch(e) { return 'none'; } })()"#,
            status: String::new(),
        },
        // ── String & Array methods ──
        ApiCheck {
            category: "String & Array Methods",
            api: "String methods",
            js_check: r#"(() => { const s = 'hello'; return (s.toUpperCase() === 'HELLO' && s.toLowerCase() === 'hello') ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "String & Array Methods",
            api: "Array methods",
            js_check: r#"(() => { const arr = [1,2,3]; return (arr.map(x => x * 2)[1] === 4 && arr.filter(x => x > 1).length === 2) ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "String & Array Methods",
            api: "Object methods",
            js_check: r#"(() => { const obj = {a: 1, b: 2}; return (Object.keys(obj).length === 2 && Object.values(obj)[0] === 1) ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        // ── Intl API ──
        ApiCheck {
            category: "Intl API",
            api: "Intl.Collator / DateTimeFormat / NumberFormat",
            js_check: r#"(() => { return (typeof Intl === 'object' && typeof Intl.Collator === 'function' && typeof Intl.DateTimeFormat === 'function' && typeof Intl.NumberFormat === 'function') ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        // ── URL enhancements ──
        ApiCheck {
            category: "URL API",
            api: "URL.parse static method",
            js_check: r#"(() => { return (typeof URL.parse === 'function' && URL.parse('https://example.com') !== null) ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        // ── Math & Date ──
        ApiCheck {
            category: "Built-in Objects",
            api: "Math object",
            js_check: r#"(() => { return (typeof Math === 'object' && typeof Math.random === 'function') ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Built-in Objects",
            api: "Date object",
            js_check: r#"(() => { return (new Date().getFullYear() > 2020) ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Built-in Objects",
            api: "RegExp support",
            js_check: r#"(() => { return ((/test/).test('test') === true) ? 'full' : 'partial'; })()"#,
            status: String::new(),
        },
        // ── NOT IMPLEMENTED ──
        ApiCheck {
            category: "WebSocket API",
            api: "WebSocket",
            js_check: r#"(() => { return typeof WebSocket === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Web Workers",
            api: "Worker",
            js_check: r#"(() => { return typeof Worker === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Cache API",
            api: "CacheStorage / Cache",
            js_check: r#"(() => { return (typeof caches === 'object' && typeof caches.open === 'function') ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "WebGPU",
            api: "GPU / GPUDevice",
            js_check: r#"(() => { return typeof GPU === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Service Workers",
            api: "ServiceWorker",
            js_check: r#"(() => { return typeof ServiceWorkerGlobalScope === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "Web Notifications",
            api: "Notification",
            js_check: r#"(() => { return typeof Notification === 'function' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
        ApiCheck {
            category: "IndexedDB",
            api: "indexedDB",
            js_check: r#"(() => { return typeof indexedDB === 'object' ? 'full' : 'none'; })()"#,
            status: String::new(),
        },
    ]
}

fn verify_node_report_coverage(node_checks: &[NodeCompatCheck]) {
    let report_modules: std::collections::BTreeSet<String> = node_checks
        .iter()
        .map(|c| c.api.to_string())
        .collect();

    let node_compat_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("missing crates dir")
        .join("runtime-core")
        .join("src")
        .join("node_compat");

    let mut runtime_modules = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(&node_compat_dir).expect("failed to read node_compat dir") {
        let entry = entry.expect("failed to read node_compat entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ts") {
            continue;
        }

        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("invalid node_compat filename");

        // Internal helper shims/entrypoints, not part of official Node built-ins matrix.
        if stem == "request" || stem == "mod" {
            continue;
        }

        let module = if stem == "timers_promises" {
            "node:timers/promises".to_string()
        } else if stem == "fs_promises" {
            "node:fs/promises".to_string()
        } else {
            format!("node:{stem}")
        };
        runtime_modules.insert(module);
    }

    let missing_in_report: Vec<String> = runtime_modules
        .difference(&report_modules)
        .cloned()
        .collect();

    assert!(
        missing_in_report.is_empty(),
        "Node modules implemented in runtime but missing from web_api_report checks: {}",
        missing_in_report.join(", ")
    );
}

fn verify_node_compat_docs_sync(node_checks: &[NodeCompatCheck]) {
    let docs_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("missing crates dir")
        .parent()
        .expect("missing workspace dir")
        .join("docs")
        .join("NODE-COMPAT.md");

    let docs = std::fs::read_to_string(&docs_path).unwrap_or_else(|e| {
        panic!(
            "failed to read NODE-COMPAT matrix at {}: {e}",
            docs_path.display()
        )
    });

    let mut parsed_levels: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();

    for line in docs.lines() {
        if !line.trim_start().starts_with("| `node:") {
            continue;
        }
        let cols: Vec<&str> = line.split('|').collect();
        if cols.len() < 4 {
            continue;
        }
        let module = cols[1].trim().trim_matches('`').to_string();
        let level = cols[2].trim().to_string();

        assert!(
            ["Full", "Partial", "Stub"].contains(&level.as_str()),
            "invalid level '{}' for module '{}' in docs/NODE-COMPAT.md",
            level,
            module
        );

        parsed_levels.insert(module, level);
    }

    let expected_modules: std::collections::BTreeSet<String> =
        node_checks.iter().map(|check| check.api.to_string()).collect();
    let documented_modules: std::collections::BTreeSet<String> =
        parsed_levels.keys().cloned().collect();

    let missing_in_docs: Vec<String> = expected_modules
        .difference(&documented_modules)
        .cloned()
        .collect();
    assert!(
        missing_in_docs.is_empty(),
        "modules missing in docs/NODE-COMPAT.md: {}",
        missing_in_docs.join(", ")
    );

    for check in node_checks {
        let documented_level = parsed_levels
            .get(check.api)
            .unwrap_or_else(|| panic!("missing '{}' in docs/NODE-COMPAT.md", check.api));
        assert_eq!(
            documented_level,
            check.profile,
            "docs/NODE-COMPAT.md level mismatch for '{}': expected '{}'",
            check.api,
            check.profile
        );
    }
}

#[test]
fn generate_web_standards_report() {
    let mut runtime = make_runtime();
    let mut checks = define_checks();
    let mut node_checks = define_node_compat_checks();
    verify_node_report_coverage(&node_checks);
    verify_node_compat_docs_sync(&node_checks);

    // Run all checks
    for check in checks.iter_mut() {
        check.status = eval_js(&mut runtime, check.js_check);
        // Normalize unexpected values
        if !["full", "partial", "none"].contains(&check.status.as_str()) {
            check.status = "none".to_string();
        }
    }

    for check in node_checks.iter_mut() {
        check.status = eval_js(&mut runtime, check.js_check);
        if !["full", "partial", "none"].contains(&check.status.as_str()) {
            check.status = "none".to_string();
        }
    }

    // Count stats
    let total = checks.len();
    let full = checks.iter().filter(|c| c.status == "full").count();
    let partial = checks.iter().filter(|c| c.status == "partial").count();
    let none = checks.iter().filter(|c| c.status == "none").count();

    // Build report
    let mut report = String::new();
    writeln!(report, "# Web Standards Compatibility Report").unwrap();
    writeln!(report).unwrap();
    writeln!(
        report,
        "> Auto-generated by `web_api_report` test. Do not edit manually."
    )
    .unwrap();
    writeln!(report).unwrap();

    // Summary
    writeln!(report, "## Summary").unwrap();
    writeln!(report).unwrap();
    writeln!(report, "| Status | Count | Percentage |").unwrap();
    writeln!(report, "|--------|------:|------------|").unwrap();
    writeln!(
        report,
        "| Full | {full} | {:.0}% |",
        full as f64 / total as f64 * 100.0
    )
    .unwrap();
    writeln!(
        report,
        "| Partial | {partial} | {:.0}% |",
        partial as f64 / total as f64 * 100.0
    )
    .unwrap();
    writeln!(
        report,
        "| None | {none} | {:.0}% |",
        none as f64 / total as f64 * 100.0
    )
    .unwrap();
    writeln!(report, "| **Total** | **{total}** | **100%** |").unwrap();
    writeln!(report).unwrap();

    // Detailed table
    writeln!(report, "## Detailed Results").unwrap();
    writeln!(report).unwrap();
    writeln!(report, "| Category | API | Status |").unwrap();
    writeln!(report, "|----------|-----|--------|").unwrap();

    for check in &checks {
        let icon = match check.status.as_str() {
            "full" => "Full",
            "partial" => "Partial",
            "none" => "None",
            _ => "?",
        };
        writeln!(report, "| {} | {} | {} |", check.category, check.api, icon).unwrap();
    }
    writeln!(report).unwrap();

    // Category summary
    writeln!(report, "## Category Summary").unwrap();
    writeln!(report).unwrap();
    writeln!(report, "| Category | Full | Partial | None | Total |").unwrap();
    writeln!(report, "|----------|-----:|--------:|-----:|------:|").unwrap();

    let categories: Vec<&str> = {
        let mut cats: Vec<&str> = checks.iter().map(|c| c.category).collect();
        cats.dedup();
        cats
    };

    // Deduplicate categories properly
    let mut unique_cats: Vec<&str> = Vec::new();
    for c in &checks {
        if !unique_cats.contains(&c.category) {
            unique_cats.push(c.category);
        }
    }

    for cat in &unique_cats {
        let cat_checks: Vec<&ApiCheck> = checks.iter().filter(|c| c.category == *cat).collect();
        let cat_full = cat_checks.iter().filter(|c| c.status == "full").count();
        let cat_partial = cat_checks.iter().filter(|c| c.status == "partial").count();
        let cat_none = cat_checks.iter().filter(|c| c.status == "none").count();
        let cat_total = cat_checks.len();
        writeln!(
            report,
            "| {cat} | {cat_full} | {cat_partial} | {cat_none} | {cat_total} |"
        )
        .unwrap();
    }
    writeln!(report).unwrap();

    // Extensions loaded
    writeln!(report, "## Runtime Extensions").unwrap();
    writeln!(report).unwrap();
    writeln!(report, "The following Deno extensions are loaded:").unwrap();
    writeln!(report).unwrap();
    writeln!(report, "- `deno_webidl` - WebIDL bindings").unwrap();
    writeln!(
        report,
        "- `deno_web` - Web APIs (Console, URL, Events, Timers, Streams, Encoding, Blob, File, etc.)"
    )
    .unwrap();
    writeln!(report, "- `deno_io` - IO primitives for runtime internals").unwrap();
    writeln!(report, "- `deno_fs` - Filesystem extension (sandboxed by permissions)").unwrap();
    writeln!(report, "- `deno_crypto` - Web Crypto API").unwrap();
    writeln!(report, "- `deno_telemetry` - OpenTelemetry support").unwrap();
    writeln!(
        report,
        "- `deno_fetch` - Fetch API (Headers, Request, Response, fetch)"
    )
    .unwrap();
    writeln!(report, "- `deno_net` - TCP/TLS networking").unwrap();
    writeln!(report, "- `deno_tls` - TLS support").unwrap();
    writeln!(report, "- `edge_node_compat` - Node.js compatibility layer (Full/Partial/Stub node:*)").unwrap();
    writeln!(report, "- `deno_node` - Minimal node shim required by runtime deps").unwrap();
    writeln!(
        report,
        "- `edge_bootstrap` - Bootstrap module that wires everything to globalThis"
    )
    .unwrap();
    writeln!(
        report,
        "- `edge_assert` - Optional test helpers extension (loaded only in CLI test mode)"
    )
    .unwrap();
    writeln!(report).unwrap();

    // Node compatibility section (explicitly separated from Web Standards checks).
    writeln!(report, "## Node API Compatibility").unwrap();
    writeln!(report).unwrap();
    writeln!(report, "This section is validated by runtime checks and is separate from Web Standards scoring.").unwrap();
    writeln!(report, "Official compatibility levels for `node:*` APIs:").unwrap();
    writeln!(report, "- `Full`: implementation considered functionally complete for the tested contract.").unwrap();
    writeln!(report, "- `Partial`: implementation works for common/runtime-safe paths with documented limitations.").unwrap();
    writeln!(report, "- `Stub`: module is importable, but unsupported methods fail deterministically when called.").unwrap();
    writeln!(report, "Stub failures use the standardized format: `[thunder] <api> is not implemented in this runtime profile`.").unwrap();
    writeln!(report, "Unsupported privileged behavior fails with deterministic errors (for example `EOPNOTSUPP`) instead of panicking or exposing host resources.").unwrap();
    writeln!(report).unwrap();
    writeln!(report, "| Module | Level | Profile | Tested Status | Notes |",).unwrap();
    writeln!(report, "|--------|-------|---------|---------------|-------|",).unwrap();
    for check in &node_checks {
        writeln!(
            report,
            "| `{}` | {} | {} | {} | {} |",
            check.api,
            node_support_tier(check.profile, &check.status),
            check.profile,
            status_label(&check.status),
            check.notes
        )
        .unwrap();
    }
    writeln!(report).unwrap();

    // write to file
    let report_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("docs")
        .join("web_standards_api_report.md");

    std::fs::write(&report_path, &report).unwrap_or_else(|e| {
        panic!("Failed to write report to {}: {e}", report_path.display());
    });

    // Also print to stdout for CI visibility
    println!("\n{report}");
    println!("Report written to: {}", report_path.display());

    // Remove unused variable warning
    let _ = categories;
}

