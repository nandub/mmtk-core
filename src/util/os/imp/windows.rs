//! Experimental Win32 implementation of MMTk's OS interfaces.
//!
//! Scope: experimental Windows x86-64 VirtualAlloc-backed mappings.
//! Complete reservations can be released; partial reservation release and
//! multi-terabyte side-metadata reservations remain compatibility concerns.
use crate::util::address::Address;
use crate::util::os::*;
use std::io::{Error, ErrorKind, Result};
use std::mem::MaybeUninit;
use windows_sys::Win32::System::Memory::{
    VirtualAlloc, VirtualFree, VirtualProtect, VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT,
    MEM_DECOMMIT, MEM_FREE, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE, PAGE_NOACCESS,
    PAGE_READWRITE,
};
use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
use windows_sys::Win32::System::Threading::{
    GetActiveProcessorCount, GetCurrentProcessId, GetCurrentThread, GetCurrentThreadId,
    SetThreadAffinityMask, ALL_PROCESSOR_GROUPS,
};

pub struct Windows;

#[cfg(all(test, target_os = "windows"))]
#[path = "windows_stress_tests.rs"]
mod windows_stress_tests;

fn info() -> SYSTEM_INFO {
    let mut value = MaybeUninit::<SYSTEM_INFO>::zeroed();
    unsafe {
        GetSystemInfo(value.as_mut_ptr());
        value.assume_init()
    }
}

fn protect(prot: MmapProtection) -> u32 {
    match prot {
        MmapProtection::ReadWrite => PAGE_READWRITE,
        MmapProtection::ReadWriteExec => PAGE_EXECUTE_READWRITE,
        MmapProtection::NoAccess => PAGE_NOACCESS,
    }
}

fn invalid(reason: &'static str) -> Error {
    Error::new(ErrorKind::InvalidInput, reason)
}

fn mmap_error(
    address: Address,
    bytes: usize,
    annotation: &MmapAnnotation<'_>,
    error: Error,
) -> MmapError {
    MmapError::new(address, bytes, annotation, error)
}

fn query(addr: usize) -> Result<MEMORY_BASIC_INFORMATION> {
    let mut record = MaybeUninit::<MEMORY_BASIC_INFORMATION>::zeroed();
    let read = unsafe {
        VirtualQuery(
            addr as *const _,
            record.as_mut_ptr(),
            std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if read == 0 {
        Err(Error::last_os_error())
    } else {
        Ok(unsafe { record.assume_init() })
    }
}

fn map_at(
    address: Address,
    bytes: usize,
    strategy: MmapStrategy,
    annotation: &MmapAnnotation<'_>,
) -> MmapResult<Address> {
    if bytes == 0
        || bytes % Windows::BYTES_IN_PAGE != 0
        || (!address.is_zero() && !address.is_aligned_to(Windows::BYTES_IN_PAGE))
    {
        return Err(mmap_error(
            address,
            bytes,
            annotation,
            invalid("invalid page alignment or size"),
        ));
    }

    // An mmap(MAP_FIXED, PROT_NONE) equivalent on an existing reservation:
    // decommit pages while retaining the reservation, allowing recommit later.
    if strategy.replace && !strategy.reserve && !address.is_zero() {
        if !matches!(strategy.prot, MmapProtection::NoAccess) {
            return Err(mmap_error(
                address,
                bytes,
                annotation,
                invalid("decommit must use NoAccess"),
            ));
        }
        if unsafe { VirtualFree(address.to_mut_ptr(), bytes, MEM_DECOMMIT) } != 0 {
            return Ok(address);
        }
        // If no mapping exists, MAP_FIXED also creates one. But on Windows the
        // address must be reservation-granularity aligned for MEM_RESERVE.
        let state =
            query(address.as_usize()).map_err(|e| mmap_error(address, bytes, annotation, e))?;
        if state.State != MEM_FREE {
            return Err(mmap_error(
                address,
                bytes,
                annotation,
                Error::last_os_error(),
            ));
        }
    }

    // A fixed replacement into an existing reservation is a commit. If the
    // address is free, MAP_FIXED means create a new mapping at that address.
    // Never replace memory that belongs to some *other* allocation.
    let flags = if address.is_zero() {
        if strategy.reserve {
            MEM_RESERVE | MEM_COMMIT
        } else {
            MEM_RESERVE
        }
    } else {
        let state =
            query(address.as_usize()).map_err(|e| mmap_error(address, bytes, annotation, e))?;
        if state.State == MEM_FREE {
            let granularity = info().dwAllocationGranularity as usize;
            if !address.is_aligned_to(granularity) {
                return Err(mmap_error(
                    address,
                    bytes,
                    annotation,
                    invalid("new fixed Windows reservation must be allocation-granularity aligned"),
                ));
            }
            if strategy.reserve {
                MEM_RESERVE | MEM_COMMIT
            } else {
                MEM_RESERVE
            }
        } else if strategy.replace && state.State == windows_sys::Win32::System::Memory::MEM_RESERVE
        {
            if !strategy.reserve {
                // An existing reserved mapping is already quarantined.
                return Ok(address);
            }
            MEM_COMMIT
        } else if strategy.replace && state.State == MEM_COMMIT {
            // MAP_FIXED has fresh-zero semantics. Decommit/recommit ensures old
            // committed contents are not silently retained.
            if unsafe { VirtualFree(address.to_mut_ptr(), bytes, MEM_DECOMMIT) } == 0 {
                return Err(mmap_error(
                    address,
                    bytes,
                    annotation,
                    Error::last_os_error(),
                ));
            }
            MEM_COMMIT
        } else {
            return Err(mmap_error(
                address,
                bytes,
                annotation,
                Error::new(
                    ErrorKind::AlreadyExists,
                    "fixed address already mapped and replace was not requested",
                ),
            ));
        }
    };

    if !strategy.reserve && !matches!(strategy.prot, MmapProtection::NoAccess) {
        return Err(mmap_error(
            address,
            bytes,
            annotation,
            invalid("uncommitted reservation must use NoAccess"),
        ));
    }
    let pointer =
        unsafe { VirtualAlloc(address.to_mut_ptr(), bytes, flags, protect(strategy.prot)) };
    if pointer.is_null() {
        let error = Error::last_os_error();
        #[cfg(debug_assertions)]
        {
            match query(address.as_usize()) {
                Ok(state) => eprintln!(
                    "MMTk Windows VirtualAlloc failure: address={address}, bytes={bytes}, flags=0x{flags:x}, replace={}, reserve={}, state=0x{:x}, allocation_base={:p}, region_size={}, error={error}",
                    strategy.replace,
                    strategy.reserve,
                    state.State,
                    state.AllocationBase,
                    state.RegionSize,
                ),
                Err(query_error) => eprintln!(
                    "MMTk Windows VirtualAlloc failure: address={address}, bytes={bytes}, flags=0x{flags:x}, replace={}, reserve={}, query_error={query_error}, error={error}",
                    strategy.replace,
                    strategy.reserve,
                ),
            }
        }
        return Err(mmap_error(address, bytes, annotation, error));
    }
    let result = Address::from_mut_ptr(pointer);
    if !address.is_zero() && result != address {
        return Err(mmap_error(
            address,
            bytes,
            annotation,
            invalid("VirtualAlloc returned unexpected address"),
        ));
    }
    Ok(result)
}

// Test-only cleanup for pages inside MMTk's process-lifetime side-metadata
// reservation. Unlike MEM_RELEASE, MEM_DECOMMIT preserves allocation ownership.
// It must not be used as a general substitute for munmap.
#[cfg(test)]
impl Windows {
    pub(crate) fn decommit_metadata_test_range(start: Address, size: usize) -> Result<()> {
        if start.is_zero()
            || size == 0
            || !start.is_aligned_to(Self::BYTES_IN_PAGE)
            || size % Self::BYTES_IN_PAGE != 0
        {
            return Err(invalid("invalid metadata decommit address or size"));
        }
        let end = start
            .as_usize()
            .checked_add(size)
            .ok_or_else(|| invalid("metadata decommit range overflow"))?;

        // Validate all pages before decommitting any. VirtualQuery may split a
        // single allocation into several commitment/protection records.
        let mut cursor = start.as_usize();
        let mut allocation_base: Option<usize> = None;
        while cursor < end {
            let record = query(cursor)?;
            let region_start = record.BaseAddress as usize;
            let region_end = region_start
                .checked_add(record.RegionSize)
                .ok_or_else(|| invalid("metadata VirtualQuery region overflow"))?;
            if region_start > cursor || region_end <= cursor || record.State == MEM_FREE {
                return Err(Error::new(
                    ErrorKind::AddrNotAvailable,
                    "metadata decommit range includes unmapped pages",
                ));
            }
            let base = record.AllocationBase as usize;
            if base == 0 || allocation_base.is_some_and(|expected| expected != base) {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    "metadata decommit spans multiple Windows reservations",
                ));
            }
            allocation_base = Some(base);
            cursor = region_end.min(end);
        }
        if unsafe { VirtualFree(start.to_mut_ptr(), size, MEM_DECOMMIT) } == 0 {
            return Err(Error::last_os_error());
        }
        Ok(())
    }
}

impl OSMemory for Windows {
    const LOG_BYTES_IN_PAGE: u8 = 12;

    fn dzmmap(
        start: Address,
        size: usize,
        strategy: MmapStrategy,
        annotation: &MmapAnnotation<'_>,
    ) -> MmapResult<Address> {
        map_at(start, size, strategy, annotation)
    }

    fn dzmmap_anywhere(
        size: usize,
        align: usize,
        strategy: MmapStrategy,
        annotation: &MmapAnnotation<'_>,
    ) -> MmapResult<Address> {
        let granularity = info().dwAllocationGranularity as usize;
        if align == 0 || !align.is_power_of_two() || size == 0 || size % Self::BYTES_IN_PAGE != 0 {
            return Err(mmap_error(
                Address::ZERO,
                size,
                annotation,
                invalid("invalid size or alignment"),
            ));
        }
        // MAP_FIXED has no meaning with a null hint: allow the OS to choose
        // a fresh base address for the quarantine (including side metadata).
        let strategy = strategy.replace(false);
        if align <= granularity {
            return map_at(Address::ZERO, size, strategy, annotation);
        }
        // Reserve at an aligned free address directly, rather than reserve a
        // large block and illegally attempt to split the Windows reservation.
        // VirtualQuery is advisory: VirtualAlloc is the authoritative check.
        // Limit attempts to avoid looping indefinitely on an exhausted VA space.
        let mut cursor: usize = granularity;
        for _ in 0..16384 {
            let candidate = match cursor.checked_add(align - 1).map(|v| v & !(align - 1)) {
                Some(v) => v,
                None => break,
            };
            if candidate.checked_add(size).is_none() {
                break;
            }
            let mut record = MaybeUninit::<MEMORY_BASIC_INFORMATION>::zeroed();
            let read = unsafe {
                VirtualQuery(
                    candidate as *const _,
                    record.as_mut_ptr(),
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            if read == 0 {
                break;
            }
            let region = unsafe { record.assume_init() };
            if region.State == MEM_FREE && region.RegionSize >= size {
                let address = unsafe { Address::from_usize(candidate) };
                if let Ok(mapped) = map_at(address, size, strategy, annotation) {
                    return Ok(mapped);
                }
            }
            let region_end = (region.BaseAddress as usize).checked_add(region.RegionSize);
            cursor = match region_end {
                Some(v) if v > candidate => v,
                _ => break,
            };
        }
        Err(mmap_error(
            Address::ZERO,
            size,
            annotation,
            Error::new(
                ErrorKind::AddrNotAvailable,
                "no suitably aligned Windows virtual-address range available",
            ),
        ))
    }

    fn dzmmap_preferred(
        start: Address,
        size: usize,
        align: usize,
        strategy: MmapStrategy,
        annotation: &MmapAnnotation<'_>,
    ) -> MmapResult<Address> {
        if strategy.replace {
            return map_at(start, size, strategy, annotation);
        }
        if !start.is_zero()
            && start.is_aligned_to(info().dwAllocationGranularity as usize)
            && align != 0
            && align.is_power_of_two()
            && start.is_aligned_to(align)
        {
            if let Ok(address) = map_at(start, size, strategy, annotation) {
                return Ok(address);
            }
        }
        Self::dzmmap_anywhere(size, align, strategy, annotation)
    }

    fn munmap(start: Address, size: usize) -> Result<()> {
        if start.is_zero()
            || !start.is_aligned_to(Self::BYTES_IN_PAGE)
            || size == 0
            || size % Self::BYTES_IN_PAGE != 0
        {
            return Err(invalid("invalid unmap address or size"));
        }
        let end = start
            .as_usize()
            .checked_add(size)
            .ok_or_else(|| invalid("unmap range overflow"))?;

        // Scan the entire request before releasing anything. Windows MEM_RELEASE
        // only releases complete reservations at their AllocationBase. FREE
        // spans are harmless for idempotent cleanup, but partial reservations
        // remain an error. Do not mistake VirtualQuery state/protection records
        // for independent allocations.
        let mut bases: Vec<usize> = Vec::new();
        let mut cursor = start.as_usize();
        let mut active_base: Option<usize> = None;
        while cursor < end {
            let record = query(cursor)?;
            let region_base = record.BaseAddress as usize;
            let region_end = region_base
                .checked_add(record.RegionSize)
                .ok_or_else(|| invalid("VirtualQuery region overflow"))?;
            if region_base > cursor || region_end <= cursor {
                return Err(invalid("VirtualQuery returned an invalid range"));
            }

            if record.State == MEM_FREE {
                // VirtualQuery may report a FREE span extending beyond the
                // requested end. Only consume the intersection.
                active_base = None;
                cursor = region_end.min(end);
                continue;
            }

            let allocation_base = record.AllocationBase as usize;
            if allocation_base == 0 || region_end > end {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    "unmap partially overlaps a Windows reservation",
                ));
            }
            if active_base != Some(allocation_base) {
                // In particular, a suffix of a reservation cannot be freed.
                if allocation_base != cursor {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        "unmap begins inside a Windows reservation",
                    ));
                }
                bases.push(allocation_base);
                active_base = Some(allocation_base);
            }
            cursor = region_end;
        }

        // The final mapped reservation could continue beyond `end` even
        // if its last in-range VirtualQuery record stops exactly at `end`.
        if let Some(last_base) = active_base {
            if let Ok(next) = query(end) {
                if next.State != MEM_FREE && next.AllocationBase as usize == last_base {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        "unmap ends inside a Windows reservation",
                    ));
                }
            }
        }

        // Windows has no atomic multi-reservation release operation. All
        // safety checks above run first; an OS error here may cause partial
        // progress and is returned to the caller.
        for base in bases {
            if unsafe { VirtualFree(base as *mut _, 0, MEM_RELEASE) } == 0 {
                return Err(Error::last_os_error());
            }
        }
        Ok(())
    }

    fn set_memory_access(start: Address, size: usize, protection: MmapProtection) -> Result<()> {
        if start.is_zero() || size == 0 {
            return Err(invalid("invalid protection range"));
        }
        let mut old = 0u32;
        if unsafe { VirtualProtect(start.to_mut_ptr(), size, protect(protection), &mut old) } == 0 {
            Err(Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn is_mmap_oom(errno: i32) -> bool {
        matches!(errno, 8 | 14 | 1455)
    }

    fn panic_if_unmapped(start: Address, size: usize) {
        assert!(!start.is_zero() && size != 0, "invalid memory range");
        let mut record = MaybeUninit::<MEMORY_BASIC_INFORMATION>::zeroed();
        let got = unsafe {
            VirtualQuery(
                start.to_mut_ptr(),
                record.as_mut_ptr(),
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        assert_ne!(got, 0, "VirtualQuery failed");
        let record = unsafe { record.assume_init() };
        assert_ne!(record.State, MEM_FREE, "range is unmapped");
        assert!(
            record.RegionSize >= size,
            "range spans multiple memory regions"
        );
    }
}

impl OSProcess for Windows {
    type ProcessIDType = u32;
    type ThreadIDType = u32;
    fn get_process_memory_maps() -> Result<String> {
        Ok("Windows memory map enumeration not yet implemented".to_string())
    }
    fn get_process_id() -> Result<Self::ProcessIDType> {
        Ok(unsafe { GetCurrentProcessId() })
    }
    fn get_thread_id() -> Result<Self::ThreadIDType> {
        Ok(unsafe { GetCurrentThreadId() })
    }
    fn get_total_num_cpus() -> CoreNum {
        (unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) }).min(u16::MAX as u32) as CoreNum
    }
    fn bind_current_thread_to_core(core_id: CoreId) {
        Self::bind_current_thread_to_cpuset(&[core_id]);
    }
    fn bind_current_thread_to_cpuset(core_ids: &[CoreId]) {
        assert!(!core_ids.is_empty(), "empty affinity mask");
        let mut mask = 0usize;
        for &id in core_ids {
            assert!(
                (id as u32) < usize::BITS,
                "processor groups are not yet supported"
            );
            mask |= 1usize << id;
        }
        assert_ne!(
            unsafe { SetThreadAffinityMask(GetCurrentThread(), mask) },
            0,
            "SetThreadAffinityMask failed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn process_and_memory_basics() {
        let _vm_test_lock = crate::util::raw_memory_freelist::tests::MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(Windows::get_process_id().unwrap() > 0);
        assert!(Windows::get_thread_id().unwrap() > 0);
        assert!(Windows::get_total_num_cpus() > 0);
        assert_eq!(info().dwPageSize, 4096);
        let annotation = MmapAnnotation::Misc {
            name: "windows-port-test",
        };
        let range =
            Windows::dzmmap_anywhere(4096, 4096, MmapStrategy::default(), &annotation).unwrap();
        Windows::panic_if_unmapped(range, 4096);
        unsafe {
            std::ptr::write_volatile(range.to_mut_ptr::<u8>(), 0x55);
        }
        Windows::munmap(range, 4096).unwrap();
    }

    #[test]
    fn aligned_reservation_commit_decommit_and_release() {
        let _vm_test_lock = crate::util::raw_memory_freelist::tests::MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let annotation = MmapAnnotation::Misc {
            name: "windows-large-alignment",
        };
        let size = 4 * 1024 * 1024;
        let addr = Windows::dzmmap_anywhere(
            size,
            size,
            MmapStrategy::QUARANTINE.replace(false),
            &annotation,
        )
        .unwrap();
        assert!(addr.is_aligned_to(size));
        let commit = MmapStrategy::default().replace(true);
        assert_eq!(
            Windows::dzmmap(addr, size, commit, &annotation).unwrap(),
            addr
        );
        unsafe {
            std::ptr::write_volatile(addr.to_mut_ptr::<u8>(), 0x5a);
        }
        let quarantine = MmapStrategy::QUARANTINE.replace(true);
        assert_eq!(
            Windows::dzmmap(addr, size, quarantine, &annotation).unwrap(),
            addr
        );
        assert_eq!(
            Windows::dzmmap(addr, size, commit, &annotation).unwrap(),
            addr
        );
        assert_eq!(
            unsafe { std::ptr::read_volatile(addr.to_mut_ptr::<u8>()) },
            0
        );
        Windows::munmap(addr, size).unwrap();
    }
    #[test]
    fn fixed_replacement_inside_reservation() {
        let _vm_test_lock = crate::util::raw_memory_freelist::tests::MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let anno = MmapAnnotation::Misc {
            name: "windows-replacement-test",
        };
        let size = 4 * 1024 * 1024;
        let base =
            Windows::dzmmap_anywhere(size, size, MmapStrategy::QUARANTINE.replace(false), &anno)
                .unwrap();
        let page = base + 4096usize;
        let rw = MmapStrategy::TEST;
        assert_eq!(Windows::dzmmap(page, 4096, rw, &anno).unwrap(), page);
        unsafe {
            std::ptr::write_volatile(page.to_mut_ptr::<u8>(), 0xA9);
        }
        assert_eq!(
            Windows::dzmmap(page, 4096, MmapStrategy::QUARANTINE, &anno).unwrap(),
            page
        );
        assert_eq!(Windows::dzmmap(page, 4096, rw, &anno).unwrap(), page);
        assert_eq!(
            unsafe { std::ptr::read_volatile(page.to_mut_ptr::<u8>()) },
            0
        );
        Windows::munmap(base, size).unwrap();
    }

    #[test]
    fn munmap_releases_multiple_complete_reservations() {
        let _vm_test_lock = crate::util::raw_memory_freelist::tests::MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let anno = MmapAnnotation::Misc {
            name: "windows-multiple-reservations",
        };
        let part = 4 * 1024 * 1024usize;
        let total = 2 * part;
        // Find a contiguous free region, then split it into two distinct
        // Windows reservations so that munmap must release both allocations.
        let whole =
            Windows::dzmmap_anywhere(total, part, MmapStrategy::QUARANTINE.replace(false), &anno)
                .unwrap();
        Windows::munmap(whole, total).unwrap();
        let first = Windows::dzmmap(whole, part, MmapStrategy::default(), &anno).unwrap();
        let second = Windows::dzmmap(whole + part, part, MmapStrategy::default(), &anno).unwrap();
        assert_eq!(first, whole);
        assert_eq!(second, whole + part);
        Windows::munmap(whole, total).unwrap();
        assert_eq!(query(whole.as_usize()).unwrap().State, MEM_FREE);
    }

    #[test]
    fn munmap_skips_free_gaps_between_complete_reservations() {
        let _vm_test_lock = crate::util::raw_memory_freelist::tests::MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let anno = MmapAnnotation::Misc {
            name: "windows-free-gap-unmap",
        };
        let chunk = 4 * 1024 * 1024usize;
        let total = 3 * chunk;
        let whole =
            Windows::dzmmap_anywhere(total, chunk, MmapStrategy::QUARANTINE.replace(false), &anno)
                .unwrap();
        Windows::munmap(whole, total).unwrap();
        let first = Windows::dzmmap(whole, chunk, MmapStrategy::default(), &anno).unwrap();
        let last =
            Windows::dzmmap(whole + 2 * chunk, chunk, MmapStrategy::default(), &anno).unwrap();
        assert_eq!(first, whole);
        assert_eq!(last, whole + 2 * chunk);
        Windows::munmap(whole, total).unwrap();
        assert_eq!(query(whole.as_usize()).unwrap().State, MEM_FREE);
        assert_eq!(query(last.as_usize()).unwrap().State, MEM_FREE);
    }

    #[test]
    fn munmap_already_free_range_is_idempotent() {
        let _vm_test_lock = crate::util::raw_memory_freelist::tests::MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let anno = MmapAnnotation::Misc {
            name: "windows-idempotent-unmap",
        };
        let size = 4 * 1024 * 1024usize;
        let base = Windows::dzmmap_anywhere(size, size, MmapStrategy::default(), &anno).unwrap();
        Windows::munmap(base, size).unwrap();
        Windows::munmap(base, size).unwrap();
        assert_eq!(query(base.as_usize()).unwrap().State, MEM_FREE);
    }

    #[test]
    fn munmap_rejects_partial_reservation_without_releasing_it() {
        let _vm_test_lock = crate::util::raw_memory_freelist::tests::MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let anno = MmapAnnotation::Misc {
            name: "windows-partial-unmap-rejection",
        };
        let size = 4 * 1024 * 1024usize;
        let whole = Windows::dzmmap_anywhere(size, size, MmapStrategy::default(), &anno).unwrap();
        let page = Windows::BYTES_IN_PAGE;
        // Both a proper prefix and a suffix must fail without releasing
        // the reservation. In particular a split VirtualQuery record is not
        // equivalent to a distinct reservation.
        assert!(Windows::munmap(whole, page).is_err());
        assert!(Windows::munmap(whole + page, size - page).is_err());
        assert_ne!(query(whole.as_usize()).unwrap().State, MEM_FREE);
        Windows::munmap(whole, size).unwrap();
    }

    #[test]
    fn metadata_test_decommit_preserves_neighbor_and_recommits_zeroed() {
        let _vm_test_lock = crate::util::raw_memory_freelist::tests::MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let annotation = MmapAnnotation::Misc {
            name: "windows-metadata-decommit",
        };
        let size = 4 * 1024 * 1024usize;
        let page = Windows::BYTES_IN_PAGE;
        let base = Windows::dzmmap_anywhere(
            size,
            size,
            MmapStrategy::QUARANTINE.replace(false),
            &annotation,
        )
        .unwrap();
        let neighbor = base + page;
        Windows::dzmmap(base, 2 * page, MmapStrategy::TEST, &annotation).unwrap();
        unsafe {
            std::ptr::write_volatile(base.to_mut_ptr::<u8>(), 0xA5);
            std::ptr::write_volatile(neighbor.to_mut_ptr::<u8>(), 0x5A);
        }
        Windows::decommit_metadata_test_range(base, page).unwrap();
        assert_eq!(query(base.as_usize()).unwrap().State, MEM_RESERVE);
        assert_eq!(query(neighbor.as_usize()).unwrap().State, MEM_COMMIT);
        assert_eq!(
            unsafe { std::ptr::read_volatile(neighbor.to_mut_ptr::<u8>()) },
            0x5A
        );
        Windows::decommit_metadata_test_range(base, page).unwrap();
        Windows::dzmmap(base, page, MmapStrategy::TEST, &annotation).unwrap();
        assert_eq!(
            unsafe { std::ptr::read_volatile(base.to_mut_ptr::<u8>()) },
            0
        );
        Windows::munmap(base, size).unwrap();
    }

    #[test]
    fn metadata_test_decommit_rejects_free_range() {
        let _vm_test_lock = crate::util::raw_memory_freelist::tests::MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let annotation = MmapAnnotation::Misc {
            name: "windows-metadata-free-decommit",
        };
        let size = 4 * 1024 * 1024usize;
        let base = Windows::dzmmap_anywhere(
            size,
            size,
            MmapStrategy::QUARANTINE.replace(false),
            &annotation,
        )
        .unwrap();
        Windows::munmap(base, size).unwrap();
        assert!(Windows::decommit_metadata_test_range(base, Windows::BYTES_IN_PAGE).is_err());
    }
}
