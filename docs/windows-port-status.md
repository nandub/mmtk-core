# Windows port — initial implementation, NOT production ready

Baseline: mmtk/mmtk-core v0.33.0, commit `7acf9b24235c40a1251c71c63cf68cbab88adcd7`.
Branch: `feature/windows-os-support` in nandub/mmtk-core.

Implemented for initial compilation/testing:
- A Rust Win32 `OS` selected by target_os windows implementing `OSMemory` and `OSProcess`.
- VirtualAlloc reservation/commit, VirtualFree full release, VirtualProtect, VirtualQuery.
- Win32 process/thread identifiers, processor count, basic affinity.
- Mandatory `--features malloc_mimalloc` on Windows to maintain consistent malloc/posix_memalign/size/free semantics.
- One Windows-only Rust unit test for process information and 4096-byte allocation lifecycle.

**Known unsolved requirements**
- VirtualAlloc2 placeholders and splitting reservations for POSIX-compatible `munmap` partial range.
- Alignments above allocation granularity, replacement of quarantined reservations, and reserve/commit transition semantics in all MMTk strategies.
- Processor groups beyond group zero and dynamic core affinity.
- Memory protection for reserved-but-not-committed regions.
- Confirming all Windows compilation failures and runtime tests (untested in generation environment).
- Replace hardcoded 4096-byte page assumption or enforce it with a startup check.
- Additional Windows-only dependencies/conditional compilation discovered during cargo check.
- CI tests on Windows and Linux and parity against MMTk's existing memory mapping tests.

This patch is **not** evidence that native Windows MMTk builds or supports a CoreCLR GC. Do not enable this fork in MMTkCLR before a native Windows `cargo check --lib`, complete MMTk test suite, and Linux regression have passed.

## Initial validation

```powershell
Set-Location 'S:\Sources\mmtk-core-windows'
.\scripts\Test-WindowsPort.ps1  # if integrated into the fork later
cargo check --lib --features malloc_mimalloc
cargo test --lib --features malloc_mimalloc
```

If additional errors emerge, preserve full logs and repair them without changing Linux behavior.

## Sources

- MMTk [OSMemory](https://github.com/mmtk/mmtk-core/blob/7acf9b24235c40a1251c71c63cf68cbab88adcd7/src/util/os/memory.rs)
- MMTk [OSProcess](https://github.com/mmtk/mmtk-core/blob/7acf9b24235c40a1251c71c63cf68cbab88adcd7/src/util/os/process.rs)
- MMTk [allocator](https://github.com/mmtk/mmtk-core/blob/7acf9b24235c40a1251c71c63cf68cbab88adcd7/src/util/malloc/library.rs)
- [VirtualAlloc](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc)
- [VirtualFree](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualfree)
- [VirtualQuery](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualquery)
- [VirtualProtect](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualprotect)
- [Windows processor groups](https://learn.microsoft.com/en-us/windows/win32/procthread/processor-groups)
