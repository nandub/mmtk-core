# Windows Port v2.7 — Bounded memory stress and qualification

## Pinned baseline
- MMTk Core v0.33.0, upstream commit `7acf9b24235c40a1251c71c63cf68cbab88adcd7`.
- Windows baseline: `9b55c695c868c46a4eb3cb2362dc37b165dac2f9`.

## Validated prior to v2.7 (2026-10-10; user-run tests)
| Check | Windows | Linux (WSL2/ext4) |
|---|---|---|
| Serial lib tests | 546 passed | 540 passed |
| Parallel lib tests | 546 passed | 540 passed |
| Full test suites | 546 unit + 4 integration passed | 540 unit + 4 integration passed |
| Strict clippy | passed | passed |

## v2.7 additions (must be tested after installation)
- Two **Windows-only** test cases, bounded at 256 serial lifecycle iterations and 8 independent parallel workers × 128 iterations each.
- Four-megabyte aligned reservations; two-page commits, partial test-only decommit, neighbor-state checks, recommit-zero checks and whole-reservation cleanup.
- PowerShell 5.1 Windows validator and Linux Bash validator with persistent test logs.
- Generated-file ignore rules and agent instructions.

### Expected new Windows unit test count
548 (if the baseline test selection and features remain unchanged). Linux remains 540.
These numbers are **expectations, not verified outcomes**.

### Windows commands
```powershell
.\scripts\Test-WindowsPort.ps1 -WhatIf
.\scripts\Test-WindowsPort.ps1 -Verbose -RepeatParallel 3
```

### Linux commands
Use a Linux-native working tree including these changes. Run `bash scripts/test-linux-port.sh`.

## Limitations
- The stress test only exercises **independently owned reservations**, not colliding allocations or shared-memory concurrency.
- No process-lifetime leak detection, exhaustive failure injection, VirtualAlloc2 placeholders, or live CoreCLR GC integration.
- Actual `OS::munmap` semantics remain unchanged.
- The v2.7 additions are a proposed patch until the user runs the validation matrix.

## References
- [Microsoft VirtualAlloc](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc)
- [Microsoft VirtualFree](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualfree)
- [Microsoft VirtualQuery](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualquery)
- [Rust std::thread::scope](https://doc.rust-lang.org/std/thread/fn.scope.html)
- [Cargo test](https://doc.rust-lang.org/cargo/commands/cargo-test.html)
- [MMTk original OSMemory interface](https://github.com/mmtk/mmtk-core/blob/7acf9b24235c40a1251c71c63cf68cbab88adcd7/src/util/os/memory.rs)
