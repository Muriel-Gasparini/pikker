# Resource measurements

Linux measurements on 2026-10-11: Rust 1.94.0, Slint 1.18.1, winit/software renderer,
173-champion / 267-role statistical catalog. Historical observations, not Windows or future-release guarantees.

| Condition | Duration | CPU (% of one core) | Mean RSS | Max sampled RSS |
| --- | --- | --- | --- | --- |
| Client absent, release in Xvfb, bootstrap finished | 60 s | 0.0333% | 36.953 MiB | 36.953 MiB |
| External synthetic client, instrumented native release UI, no events | 60 s | 0.0167% | 42.883 MiB | 42.883 MiB |
| Standard release minimized in Wayland/XWayland | 60 s | 0.0167% | 32.189 MiB | 32.191 MiB |

Synthetic client, Xvfb and compositor were excluded. The instrumented test linked the testing backend
but rendered with winit/software; it is not the Windows EXE. Minimized samples required Iconic on all
60 checks; earlier attempts without sustained minimization were rejected.

Real meta bootstrap: one 60-second sample used 0.2333% of one core; process VmHWM reached 36.980 MiB.
Downloads/startup are not idle. Ranking-only benchmark p95: 0.0132 ms for 173 champions (1,000 iterations),
0.5140 ms for 3,000 synthetic records. This does not measure recommendation quality.

Reproduce with `sh scripts/measure-process.sh PID 60` and
`cargo run --release --locked --no-default-features --example benchmark`.
For minimized measurements set `PIKKER_MEASURE_WINDOW` and the correct `DISPLAY`.
No Windows RSS/CPU, GPU/VRAM or LoL FPS impact has been measured; zero resource use is not promised.
