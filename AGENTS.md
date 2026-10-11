# Agent instructions — MMTk Core Windows port

## Scope
- Fork: `nandub/mmtk-core`, branch `feature/windows-os-support`.
- Baseline: MMTk Core 0.33.0 at upstream `7acf9b24235c40a1251c71c63cf68cbab88adcd7`.
- Native Windows backend: `src/util/os/imp/windows.rs`; test-only stress tests: `windows_stress_tests.rs`.
- Do not equate passing core tests with a functioning CoreCLR GC binding.

## Changes
- Keep Linux and Windows behavior separate using precise `cfg` selection.
- Respect Win32 `VirtualFree`: `MEM_RELEASE` requires allocation base and zero size; `MEM_DECOMMIT` preserves the reservation.
- Never decommit arbitrary memory or use a test-only decommit helper in production paths.
- Exercise Windows memory APIs only on allocations owned by the test.
- Do not introduce `unsafe` without explaining invariants and bounded memory ownership.
- Avoid overlapping-reservation concurrent tests without explicit synchronization.

## Validation
- Windows PowerShell 5.1: `./scripts/Test-WindowsPort.ps1 -Verbose` (native PowerShell syntax: `.\scripts\Test-WindowsPort.ps1`).
- Linux: `bash scripts/test-linux-port.sh` on the ext4 WSL filesystem, not DrvFs.
- Run formatting, `cargo check`, strict clippy, serial and parallel full suites.
- Record exact test counts and dates; do not claim passing tests that weren't executed.

## Documentation and provenance
- Update `docs/windows-port-status.md` and affected READMEs alongside changes.
- Add links to primary APIs and upstream source for implementation decisions.
- Explain new platform limits, test coverage, untested scenarios, and rollback process.
- Keep generated logs, backups and Cargo target directories out of commits.
