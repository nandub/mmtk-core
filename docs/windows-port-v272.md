# MMTk Windows qualification v2.7.2

## Scope

This is a test-isolation and failure-diagnostics update to the **experimental**
Windows backend. It does **not** change production allocation policy, memory
ownership, or synchronization rules. v2.7.2 does not claim production GC
qualification.

## Root-cause hypothesis and evidence

A Windows `cargo test --lib --features malloc_mimalloc -- --test-threads=4`
run intermittently failed with `ERROR_INVALID_ADDRESS` (487) when
`RawMemoryFreeList` attempted to reserve-and-commit address `0x6f000000`.
Other parallel runs terminated with `0xc0000005`, whose specific cause has not
been established. The RawMemoryFreeList tests use a mutex to serialize their
own fixed-address mappings; v2.7's stress tests created multiple *automatic*
reservations concurrently without taking that mutex. The operating system
can select arbitrary free ranges, so the tests could interfere with a fixed
address mapping. This is a plausible interaction, **not a proven diagnosis**.

## Changes

- Share the existing RawMemoryFreeList test mutex with both Windows stress tests.
  Each stress test holds it for its whole body, including its internal workers.
  Other unrelated tests do not participate in this lock; investigate any
  remaining crashes rather than assuming isolation is complete.
- On a failed `VirtualAlloc` operation, capture the original Win32 error
  immediately, then under debug assertions emit a subsequent `VirtualQuery`
  snapshot (state, region size, allocation base, flags). The snapshot is not
  an atomic ownership proof and may already differ from the failed call state.
- Replace `Start-Process -Wait` with a timeout-bounded wait against the Cargo
  process alone. Separate stdout/stderr capture avoids Windows PowerShell 5.1
  converting ordinary Cargo stderr into terminating `NativeCommandError`.
- Back up edited files and retain a `-WhatIf` installation path.

## Windows verification

```powershell
.\Install-MMTkWindowsV272.ps1 -RepositoryPath 'S:\Sources\mmtk-core-windows' -WhatIf
.\Install-MMTkWindowsV272.ps1 -RepositoryPath 'S:\Sources\mmtk-core-windows' -Verbose
Set-Location 'S:\Sources\mmtk-core-windows'
cargo fmt --all
.\scripts\Test-WindowsPort.ps1 -RepeatParallel 30 -StepTimeoutSeconds 600 -Verbose
```

If any run fails, retain the log for that iteration; WER crash dumps are
expected only for an unhandled native crash, not for ordinary Rust assertions.
After Windows qualification, run the existing `scripts/test-linux-port.sh` in
WSL2/ext4 to check Linux compatibility. Do not commit if intermittent crashes
remain.

## References

- Microsoft: [VirtualAlloc](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc)
- Microsoft: [VirtualFree](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualfree)
- Microsoft: [VirtualQuery](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualquery)
- Rust: [Test harness options](https://doc.rust-lang.org/rustc/tests/index.html)
- Microsoft: [Windows Error Reporting LocalDumps](https://learn.microsoft.com/en-us/windows/win32/wer/collecting-user-mode-dumps)
