# Logging

**Status:** Implemented  
**Owner:** `crates/engine::logging` installs the process-wide `log` facade backend  
**Source:** [engine logger](../../crates/engine/src/logging.rs), [engine startup](../../crates/engine/src/lib.rs), [renderer log calls](../../crates/renderer/src/lib.rs)

## Purpose and boundary

Logging writes timestamped diagnostic records from the engine, renderer, and applications. It owns the default stderr formatter and log-level filter. It does not own application state or capture files.

## State and invariants

The backend is stateless after installation. The `log` facade allows only one process-wide logger; `init_logging` installs the engine backend if none exists and leaves an application-provided backend in place. The configured maximum level is applied once when the engine backend is installed.

## Public contract

`labyrinth::run` calls `init_logging` before creating the event loop or invoking application setup. Applications can use the re-exported `trace!`, `debug!`, `info!`, `warn!`, and `error!` macros. Structured key-values use the `log` crate's key-value syntax, for example:

```rust
labyrinth::info!(graphics_api:? = settings.graphics_api; "Initializing renderer");
```

The default filter is `info`. `LABYRINTH_LOG` accepts `trace`, `debug`, `info`, `warn`, `error`, or `off`. Records use UTC timestamps with millisecond precision, level, target, optional source location, message, and structured fields. The default backend writes to stderr. An application that installs another `log` backend before `run` retains control of formatting and output.

## Lifecycle

`run` initializes logging before window setup, then engine and renderer messages use the same facade as application messages. Initialization is safe to call more than once; subsequent calls leave the already-installed backend unchanged. The logger flush hook is a no-op because each record is written as one line to a locked stderr handle.

## Connections

| Other system | Direction | Data or command | When | Failure behavior |
| --- | --- | --- | --- | --- |
| [Windowing](windowing.md) | Windowing → Logging | Calls `init_logging` before event-loop and application startup | Each `run` call | An existing application logger remains installed |
| Environment | Environment → Logging | `LABYRINTH_LOG` maximum level | Logger installation | Unrecognized values use the `info` default |
| [Rendering](rendering.md) | Rendering → Logging | Renderer startup and first-present records through the `log` facade | Renderer creation and first presented frame | Records follow the installed backend and level filter |
| [Debug Capture](capture.md) | Debug Capture → Logging | Capture and performance-report output paths | After a capture or report is written | Records follow the installed backend and level filter |
| Applications | Application → Logging | Level macros with message and optional structured key-values | Whenever application code emits a record | Records below the configured maximum level are discarded |

## Verification and open questions

Verification: `cargo check -p labyrinth -p renderer -p genos`, `cargo fmt --all -- --check`, and `git diff --check`.
