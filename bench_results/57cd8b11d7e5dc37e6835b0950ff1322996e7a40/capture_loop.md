# capture_loop bench result

Commit: `57cd8b11d7e5dc37e6835b0950ff1322996e7a40`
Date: 2026-05-23

## Windows GetProcessTimes run

Command:

```text
SYNAPSE_CAPTURE_BENCH_SECONDS=30 C:\Temp\synapse-bench\capture_loop.exe --quiet --noplot
```

Result:

```text
capture_loop_steady_state source=GetProcessTimes duration_secs=30.001 cpu_percent=0.0439 frames_captured=1318 frames_dropped=0 frames_consumed=1318 channel_len=0
```

Budget: capture with consumer attached at 60 fps must be `<= 2%` normalized CPU.
Observed normalized CPU: `0.0439%`.

## WSL synthetic readback run

Command:

```text
SYNAPSE_CAPTURE_BENCH_SECONDS=30 cargo bench -p synapse-capture --bench capture_loop -- --quiet
```

Result:

```text
capture_loop_60fps_start_stop time: [66.287 ms 67.972 ms 70.388 ms]
capture_loop_steady_state source=/proc/self/stat duration_secs=30.001 cpu_percent=0.0111 frames_captured=1790 frames_dropped=0 frames_consumed=1790 channel_len=0
```
