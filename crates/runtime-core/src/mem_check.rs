use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Tracks heap memory usage for an isolate and enforces limits.
pub struct MemCheck {
    limit_bytes: usize,
}

impl MemCheck {
    pub fn new(limit_bytes: usize) -> Self {
        Self { limit_bytes }
    }

    /// Check if the isolate has exceeded its memory limit.
    /// Returns `(used_bytes, exceeded)`.
    pub fn check(&self, isolate: &mut deno_core::v8::Isolate) -> (usize, bool) {
        let stats = isolate.get_heap_statistics();
        let used = stats.used_heap_size() + stats.external_memory();
        let exceeded = self.limit_bytes > 0 && used >= self.limit_bytes;
        (used, exceeded)
    }

    pub fn limit_bytes(&self) -> usize {
        self.limit_bytes
    }
}

/// State for the near-heap-limit callback.
/// This is passed as data pointer to the V8 callback.
pub struct HeapLimitState {
    /// Number of times the callback has been invoked.
    pub callback_count: AtomicUsize,
    /// Maximum heap size to allow (in bytes).
    pub max_heap_bytes: usize,
    /// Name of the function (for logging).
    pub function_name: String,
    /// Flag indicating the isolate should terminate due to memory.
    pub should_terminate: std::sync::atomic::AtomicBool,
    /// Thread-safe V8 handle to terminate execution from callback.
    pub isolate_handle: deno_core::v8::IsolateHandle,
}

impl HeapLimitState {
    pub fn new(
        max_heap_bytes: usize,
        function_name: String,
        isolate_handle: deno_core::v8::IsolateHandle,
    ) -> Self {
        Self {
            callback_count: AtomicUsize::new(0),
            max_heap_bytes,
            function_name,
            should_terminate: std::sync::atomic::AtomicBool::new(false),
            isolate_handle,
        }
    }
}

/// Near heap limit callback for V8.
///
/// Called when V8 is about to exceed the heap limit. The callback can:
/// 1. Return a larger limit to allow more memory (first call - give a last chance)
/// 2. Return the same limit to trigger OOM (second call - terminate)
///
/// # Safety
/// This function is called from V8's allocator and must be careful with allocations.
pub extern "C" fn near_heap_limit_callback(
    data: *mut std::ffi::c_void,
    current_heap_limit: usize,
    _initial_heap_limit: usize,
) -> usize {
    // Safety: data is a raw pointer to HeapLimitState that we created
    let state = unsafe { &*(data as *const HeapLimitState) };
    let count = state.callback_count.fetch_add(1, Ordering::SeqCst);

    if count == 0 {
        // First call - log warning and give a small extension (10% more)
        let extension = current_heap_limit / 10;
        let new_limit = current_heap_limit + extension;

        // Use eprintln as tracing might allocate
        eprintln!(
            "WARN: function '{}' approaching heap limit ({} bytes used, limit {}). Allowing {} more bytes.",
            state.function_name,
            current_heap_limit,
            state.max_heap_bytes,
            extension
        );

        new_limit
    } else {
        // Second call - memory is still growing, mark for termination
        state.should_terminate.store(true, Ordering::SeqCst);
        state.isolate_handle.terminate_execution();

        eprintln!(
            "ERROR: function '{}' exceeded heap limit after extension. Marking for termination.",
            state.function_name
        );

        // Return a larger limit to avoid process-level fatal OOM while
        // terminate_execution unwinds the current JS execution.
        current_heap_limit.saturating_mul(2)
    }
}

/// Atomic counter for tracking memory across isolates.
pub struct GlobalMemoryTracker {
    total_used: AtomicU64,
}

impl GlobalMemoryTracker {
    pub fn new() -> Self {
        Self {
            total_used: AtomicU64::new(0),
        }
    }

    pub fn add(&self, bytes: u64) {
        self.total_used.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn sub(&self, bytes: u64) {
        self.total_used.fetch_sub(bytes, Ordering::Relaxed);
    }

    pub fn total(&self) -> u64 {
        self.total_used.load(Ordering::Relaxed)
    }
}

impl Default for GlobalMemoryTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "mem_check_test.rs"]
mod tests;
