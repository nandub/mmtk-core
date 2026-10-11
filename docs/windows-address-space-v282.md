# Windows address-space diagnostics (v2.8.2)

## Scope and evidence

A standalone, package-free .NET 10 x64 diagnostic assists MMTk Core Windows backend qualification. **It does not certify MMTkCLR or CoreCLR compatibility**. No production Rust sources are changed.

`inspect` reads `GetSystemInfo` (page size, allocation granularity and application-address range) without reserving pages. `probe` (explicit opt-in) requests **one bounded reservation** through `VirtualAlloc(MEM_RESERVE)`, checks ownership with `VirtualQuery`, commits two small windows, decommits/recommits one, checks zeroing and neighbor isolation, releases the *entire* reservation with `VirtualFree(MEM_RELEASE)` and verifies `MEM_FREE`.

Safety limits: 1–4096 MiB reservation and 4–1024 KiB commitment per window. The probe is process-local, writes only its own committed memory, and attempts cleanup in `finally`. A failed cleanup reports an error to stderr; after the diagnostic process exits, its address space is reclaimed by Windows. A successful probe **does not** demonstrate collision-free coexistence with CoreCLR, and failure to reserve a requested size does not establish a universal capacity limit.

## Actual MMTk source facts

As reviewed on the `feature/windows-os-support` branch:

- `src/util/metadata/side_metadata/mod.rs`: `initialize_side_metadata<VM>` gathers VM side-metadata specifications, calls `set_vm_side_metadata_specs`, and invokes `initialize_side_metadata_base` with `options.side_metadata_base_address`.
- `src/util/metadata/side_metadata/helpers.rs`: contiguous metadata address translation depends on `SideMetadataSpec.log_num_of_bits`, `log_bytes_in_region`, and `get_starting_address()`.
- `src/util/os/imp/windows.rs`: the experimental Windows backend maps through `VirtualAlloc` and releases through `VirtualFree`; side-metadata test cleanup uses `MEM_DECOMMIT` because an interior `MEM_RELEASE` cannot free a partial Windows reservation.

These source references are observations from the repository, **not** computed heap-size requirements. This diagnostic deliberately does *not* claim to derive metadata reservation sizes from the above specifications. Calculating MMTk's exact metadata footprint requires resolving actual runtime binding specs, object-space layout, and initialization options; a subsequent integration harness must measure those values.

## Run on Windows (PowerShell 5.1)

```powershell
Set-Location 'S:\Sources\mmtk-core-windows'
dotnet build .\tools\MMTk.AddressSpaceDiagnostics\MMTk.AddressSpaceDiagnostics.csproj -c Release
dotnet run --no-build -c Release --project .\tools\MMTk.AddressSpaceDiagnostics -- inspect --json
dotnet run --no-build -c Release --project .\tools\MMTk.AddressSpaceDiagnostics -- probe --size-mib 1024 --commit-kib 64 --json
```

For a repeatable JSON evidence file:

```powershell
$reportDir = Join-Path $env:LOCALAPPDATA 'MMTk\AddressSpace'
New-Item -ItemType Directory -Force -Path $reportDir | Out-Null
$reportPath = Join-Path $reportDir ('v282-{0}.json' -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
dotnet run --no-build -c Release --project .\tools\MMTk.AddressSpaceDiagnostics -- probe --size-mib 1024 --commit-kib 64 --json | Set-Content -LiteralPath $reportPath -Encoding UTF8
if ($LASTEXITCODE -ne 0) { throw 'Address-space probe failed' }
```

The probe report records addresses, requested reservation size, measured allocation granularity, original allocation base, zeroing and neighbor checks, and whether the released region was observed as `MEM_FREE`. Addresses are diagnostic and should not be treated as stable across runs.

## Acceptance criteria

1. `inspect --json` succeeds without allocating virtual memory.
2. `probe --size-mib 1024 --commit-kib 64 --json` completes with `RecommitZeroed`, `NeighborPreserved` and `ReleasedAndObservedFree` all `true`.
3. Invalid or out-of-bounds probe arguments exit nonzero.
4. Existing Windows and Linux MMTk qualification continues to pass.
5. Neither diagnostic mode is represented as proof of binding compatibility.

## Remaining blockers

- Determine real side-metadata base and capacity from selected VM/policy specifications.
- Compare MMTk and CoreCLR's actual requested heap and metadata ranges **within the same process**.
- Validate reservation ownership and concurrent modification semantics in production, not solely under test mutexes.
- Confirm the VMBinding ABI, object layout, GC root scanning, write barriers, and NoGC bootstrap.

## External technical sources

- Microsoft, [VirtualAlloc](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc): reservation and commitment semantics.
- Microsoft, [VirtualFree](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualfree): complete release and decommit semantics.
- Microsoft, [VirtualQuery](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualquery): memory region inspection.
- Microsoft, [GetSystemInfo](https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-getsysteminfo): system address space and granularity.
