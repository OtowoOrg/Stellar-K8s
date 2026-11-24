# cache-plugin

A **partial** implementation of issue #4 ("Implement WebAssembly Fail-Open
Caching Layer for Soroban RPC State Reads"). This delivers the piece of
that 200-point issue that's genuinely self-contained and verifiable on its
own — the cache algorithm itself — and is explicit below about the large
remainder that is not attempted here.

## What's implemented

- **`src/cache.rs`** — a bounded LRU + TTL cache engine (`LruTtlCache`),
  with capacity clamped to a hard ceiling (`MAX_CAPACITY = 10_000`) so a
  misconfigured size can't grow memory use without bound, and a
  `capacity: 0` "disabled" mode that always misses instead of erroring.
  8 unit tests cover LRU eviction order, TTL expiry (including the exact
  boundary), refreshing an existing key, the zero-capacity/zero-TTL edge
  cases, and the hard capacity clamp.
- **`src/lib.rs`** — a Wasm plugin entry point (`cache_batch`) using the
  same `read_input`/`write_output`/`log_message` host-function ABI as
  `examples/plugins/example-validator`, so it's reachable the same way any
  other plugin in the Custom Validation Plugin system is. It wraps every
  cache operation in `std::panic::catch_unwind` and turns a panic (or a
  disabled/misconfigured cache) into a `Bypass` result instead of letting
  it propagate. 4 more tests exercise this at the plugin-request level,
  including one that triggers a **real** `panic!` inside the cache call
  path and asserts the batch keeps going and reports `bypass` for that op
  rather than aborting.
- Compiles to a **166KB** `.wasm` binary (`cargo build --target
  wasm32-wasip1 --release`), comfortably under the issue's 2MB ceiling.

Run it:

```sh
cd wasm-plugins/cache-plugin
cargo test                                    # 12 tests, native target
cargo build --target wasm32-wasip1 --release  # produces the real .wasm
```

(`rustup target add wasm32-wasip1` first if it isn't installed — this is
the current name for what `examples/plugins/example-validator`'s doc
comment still calls `wasm32-wasi`, and it's what `wasmtime_wasi::preview1`
in `src/webhook/runtime.rs` expects.)

## What's explicitly NOT implemented

This is the honest accounting of the other ~85% of the issue:

1. **No RPC interception.** There is no `soroban-rpc` integration layer in
   this repo to hook into, and none is added here. This crate is a cache
   *engine* plus a *plugin entry point*, not a request-routing proxy sitting
   in front of Soroban RPC's read path.
2. **No cross-request persistence — and this is an architectural blocker,
   not just missing glue.** `WasmRuntime::execute_sync`
   ([`src/webhook/runtime.rs`](../../src/webhook/runtime.rs)) creates a
   fresh `wasmtime::Store` and instantiates the plugin module *from
   scratch on every single call*. An in-Wasm cache's whole value
   proposition is surviving across many separate reads, but under that
   execution model every invocation starts with blank linear memory — the
   cache would be empty on every call, always miss, and provide zero
   latency benefit. Making a Wasm-sandboxed cache actually useful requires
   first changing `WasmRuntime` to keep one `Store`/`Instance` alive across
   calls for a given plugin (or accepting that the cache lives host-side
   instead of inside the sandbox, which is a materially different design
   than what the issue asks for). That change is out of scope here; this
   plugin's request format (a *batch* of ops replayed against one
   freshly-built cache in a single call) is the most the current
   architecture can demonstrate.
3. **No ConfigMap wiring.** `RequestConfig` in `lib.rs` accepts `capacity`
   and `ttlSeconds` as part of the JSON request today (see "Configuration"
   below for the shape), but nothing reads them from a Kubernetes
   ConfigMap — that plumbing lives in the operator's reconciler /
   `PluginConfig` loading path, which this change doesn't touch.
4. **No 10k req/s benchmark, no latency numbers.** Proving reduced RPC
   load needs the RPC integration from point 1 to exist first; there is
   nothing to load-test yet.
5. **LRU recency tracking is O(n) per touch** (a plain `Vec<K>`, see the
   doc comment on `LruTtlCache`), an intentional simplicity tradeoff for
   this slice. A version built to actually handle high request volume
   should replace it with an O(1) structure (e.g. an intrusive linked
   hashmap) behind the same public API.

## Configuration (today: request-level; ConfigMap wiring is future work)

Every call to `cache_batch` includes its own config, matching the TTL/size
knobs the issue calls out:

```json
{
  "config": { "capacity": 1024, "ttlSeconds": 30 },
  "ops": [
    { "op": "put", "key": "contract/getBalance/GABC...", "value": "1000", "now": 1735000000 },
    { "op": "get", "key": "contract/getBalance/GABC...", "now": 1735000005 }
  ]
}
```

- `capacity` — max entries; clamped server-side to `cache::MAX_CAPACITY`
  (10,000) no matter what's requested. `0` disables the cache (every `get`
  reports a miss).
- `ttlSeconds` — seconds before an entry expires; `0` means entries never
  expire on their own (only capacity-based LRU eviction applies).
- `now` — the cache never reads a system clock itself; every op supplies
  its own timestamp. This keeps the engine deterministic/testable and
  sidesteps depending on a WASI clock import being available in whatever
  host eventually embeds this. **When wired up for real**, the natural
  place for `capacity`/`ttlSeconds` to come from a cluster operator's
  ConfigMap is the existing `PluginConfig`/`PluginMetadata.limits` loading
  path in `src/webhook/types.rs` and `src/webhook/runtime.rs` — not
  addressed by this change.

Response shape (this is the actual `serde_json` output for the request
above, not a hand-written approximation):

```json
{
  "results": [
    "ok",
    { "hit": { "value": "1000" } }
  ],
  "any_bypassed": false
}
```

A `{"bypass": {"reason": "..."}}` result means: treat this as a cache miss
and go to the real RPC node. That is the fail-open contract this crate
provides; everything upstream of it (actually routing to Soroban RPC on a
bypass) is outside this change.
