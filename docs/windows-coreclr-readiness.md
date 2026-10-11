# MMTk Core Windows → CoreCLR readiness (v2.8)

## Verified local baseline (user-supplied logs, October 10, 2026)

- Git commit `16c1279d3a` on `feature/windows-os-support`.
- Windows: format, check, strict Clippy, serial cargo tests and 30 four-thread repeats completed with exit code 0.
- WSL2/Linux: 540 library tests plus 4 integration tests pass serially and in parallel.
- These runs validate tested MMTk-Core behavior, **not** a .NET 10 CoreCLR integration.

## Scope

This checkpoint performs *inventory and design review only*. No CoreCLR source, Rust backend code, runtime API, or GC algorithm is modified. `scripts/Test-MMTkWindowsReadiness.ps1` produces reproducible JSON evidence, with source hashes and precise repository commit. Marker presence is not semantic verification. `-RunQualification` optionally calls the pre-existing Windows qualification runner; without it, no tests execute.

## Contract review matrix

| Area | Evidence needed | Gate |
| --- | --- | --- |
| Side metadata | Maximum virtual reservation on supported Windows x64 and runtime-selected metadata base; no overlap with CoreCLR regions | Startup allocation/commit tests |
| Fixed mapping | `MEM_FREE` vs `MEM_RESERVE` vs `MEM_COMMIT`, `AllocationBase`, allocation granularity, ownership | Explicit success and failure-path tests |
| Release | Whole reservation only, preservation of adjacent regions, partial release rejected | Regression tests |
| Concurrency | Multi-threaded map/unmap of owned reservations; no double release, no TOCTOU corruption | Isolated deterministic concurrency harness |
| Zero semantics | Fresh zero contents after decommit/recommit and replacement | Readback tests |
| Protection | Guard/no-access/read-write transitions, error handling | VirtualProtect tests |
| Binding ABI | Actual `VMBinding` implementation, managed object model, slots, barriers and roots | Binding build + contract tests |
| CoreCLR handshake | GC allocation, collections, suspension, runtime callbacks, pinning, finalization | NoGC smoke test followed by GC plan |

**Do not infer CoreCLR compatibility solely from the successful MMTk Core unit suite.** Runtime integration needs its own binding and end-to-end tests.

## Proposed durable sequence

1. Inventory current checkout with `Test-MMTkWindowsReadiness.ps1`; save JSON and check the exact commit.
2. Review MMTk heap and side-metadata virtual memory layout against CoreCLR's Windows address-space policies.
3. Add targeted allocation-boundary, reservation-ownership, and concurrency tests to the Rust backend.
4. Define binding ABI contracts and runtime callback responsibilities without modifying CoreCLR initially.
5. Follow MMTk porting guidance: bootstrap with `NoGC` to validate allocation and mutator lifecycle.
6. Bring up .NET 10 CoreCLR runtime with the binding and collect traceable evidence before enabling a collecting GC.

## Authoritative references

- MMTk porting guide, *Starting a Port: NoGC*: https://docs.mmtk.io/portingguide/howto/nogc.html
- MMTk `ObjectModel` contract and side-metadata layout: https://docs.mmtk.io/api/mmtk/vm/trait.ObjectModel.html
- MMTk 64-bit side-metadata layout: https://docs.mmtk.io/api/mmtk/util/metadata/index.html
- .NET CoreCLR build workflow: https://github.com/dotnet/runtime/blob/main/docs/workflow/building/coreclr/README.md
- Windows VirtualAlloc documentation: https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc
- Windows VirtualFree documentation: https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualfree
- Windows VirtualQuery documentation: https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualquery
