# Manual FSV - Issue #1695 Forge GPU Routing

Date: 2026-07-16

## Scope

Issue #1695 required profile-driven Forge GPU work for Synapse's actual Calyx workloads. The implemented change ships first-class `Backend::knn` and `Backend::paired_cosine`, wires Loom `agreement_batch_gpu` through the paired batch operation, and documents measured CPU routing for deferred ops that did not justify a GPU port.

## MCP Preconditions

Real client/tool-surface precondition was satisfied through the configured Codex `mcp__synapse` client. Tool discovery exposed the `health`, `setup`, `storage`, and `shell` tools from the `synapse` MCP server.

Physical runtime readbacks:

- `health`: `ok=true`, daemon PID `75204`, bind `127.0.0.1:7700`, `tool_count=40`, tool surface SHA-256 `e20cb889682709ec22f9b571f043da594ffe1d6c40168566235fe45d4654bb12`.
- `health` Calyx math: backend `cuda`, device `NVIDIA GeForce RTX 5090`, VRAM `32606` MiB, probe status `ok`.
- `setup status`: token file exists at `C:\Users\hotra\AppData\Roaming\synapse\token.txt`, daemon run-current file exists, Codex config mentions Synapse and bearer-token env.
- `storage summary`: backend `rocksdb`, `CF_EPISODES=3638`, `CF_TIMELINE=148568`, pressure `Normal`.
- `nvidia-smi` before trigger: `NVIDIA GeForce RTX 5090, 32607 MiB total, 4248 MiB used, 3% GPU util`.

## Source of Truth

Behavior SoT for this manual run:

`C:\Users\hotra\AppData\Local\Temp\synapse1695-manual\report.json`

The report file is outside the repo and is not committed. The real MCP trigger was `mcp__synapse.shell operation=run command=cargo ...` from `C:\Users\hotra\AppData\Local\Temp\synapse1695-manual`, importing the local Calyx crates from `C:\code\Synapse\calyx\crates`.

Before trigger readback:

```json
{"exists":false,"path":"C:\\Users\\hotra\\AppData\\Local\\Temp\\synapse1695-manual\\report.json"}
```

## Trigger

Real MCP tool call:

```text
mcp__synapse.shell operation=run
command=cargo
args=["run","--quiet","--","C:\\Users\\hotra\\AppData\\Local\\Temp\\synapse1695-manual\\report.json"]
working_dir=C:\Users\hotra\AppData\Local\Temp\synapse1695-manual
env.CUDA_PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3
```

The final trigger exited `0`. An earlier trigger failed before exercising Forge because the temporary report structs used `&'static str` labels while deserializing the report; that trigger-source bug was corrected and the SoT was cleared/read again before the final trigger.

## Separate SoT Read After Trigger

After-trigger file readback:

```json
{
  "path": "C:\\Users\\hotra\\AppData\\Local\\Temp\\synapse1695-manual\\report.json",
  "length": 4374,
  "sha256": "9AEBA3FDD4B5BC469159EDBA042EA953ACE2D9B331A0500170EACE586F3049F1",
  "device": "NVIDIA GeForce RTX 5090",
  "happy_pass": true,
  "happy_indices": "0,1,2,1",
  "max_pair_delta": 3.5762786865234375e-7,
  "cpu_elapsed_us": 359,
  "cuda_elapsed_us": 5618,
  "edge_count": 4,
  "edges": "edge_empty_knn:True;edge_invalid_query_shape:True;edge_zero_norm_pair:True;edge_k_exceeds_cuda_topk_bound:True"
}
```

## Scenario Evidence

Happy path:

- Before: report did not exist; `knn_rows=0`; `paired_cosine_scores=0`.
- Triggered operations: CPU and CUDA `Backend::knn` on two known queries/four known candidates; CPU and CUDA Loom agreement over 2,048 deterministic pairs.
- Expected KNN indices: `[0, 1, 2, 1]`.
- After: CPU and CUDA KNN both stored indices `[0, 1, 2, 1]` and equal scores `[1.0, 0.9701424837112427, 1.0, 0.24253562092781067]`; paired cosine counts both `2048`; max CPU/CUDA pair delta `3.5762786865234375e-7`; CPU timing `359 us`; CUDA timing `5618 us`.

Edge cases:

1. Empty KNN
   - Before: `query_count=0`, `candidate_count=0`, requested `k=4`.
   - After: `query_count=0`, effective `k=0`, `indices=[]`, `scores=[]`.
2. Invalid query shape
   - Before: `query_len=3`, `query_count=1`, `dim=4`, `candidate_len=16`.
   - After: error `CALYX_FORGE_SHAPE_MISMATCH expected=[1, 4] got=[3]`, remediation `knn queries length does not match rows*cols`.
3. Zero-norm paired cosine
   - Before: left vector `[0.0,0.0,0.0,0.0]`, right vector `[1.0,0.0,0.0,0.0]`.
   - After: Loom failed closed with `CALYX_LOOM_FORGE_UNAVAILABLE` wrapping Forge `CALYX_FORGE_NUMERICAL_INVARIANT ... zero-norm sentinel output`.
4. CUDA exact top-k bound
   - Before: `candidate_count=1025`, requested `k=1025`, CUDA exact-top-k max `1024`.
   - After: error `CALYX_FORGE_SHAPE_MISMATCH expected=[1024] got=[1025]`, remediation `cuda knn uses exact topk and is bounded to k <= 1024`.

## Structural Checks

Supporting checks run after code changes:

```text
cargo check --manifest-path calyx\Cargo.toml -p calyx-forge --features cuda
cargo check --manifest-path calyx\Cargo.toml -p calyx-loom --features cuda
cargo fmt --manifest-path calyx\Cargo.toml --all --check
cargo clippy --manifest-path calyx\Cargo.toml -p calyx-forge -p calyx-loom --features cuda --all-targets
```

All completed successfully. These are compile/lint/format checks only, not FSV.

## Result

Manual FSV accepted for #1695. The real CUDA device executed the new Forge KNN and paired-cosine paths through a real MCP `shell` trigger, and the separate report-file SoT read contains the expected happy-path outputs plus fail-closed edge-case state.
