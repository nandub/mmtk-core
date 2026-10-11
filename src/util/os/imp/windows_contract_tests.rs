//! Windows v2.8.1 virtual-memory behavioral contract regressions.
//! These tests use the existing cross-module mutex to avoid interference from
//! unrelated tests mapping fixed addresses. Parallelism within a test is real.
use super::*;

fn isolate() -> std::sync::MutexGuard<'static, ()> {
    crate::util::raw_memory_freelist::tests::MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn annotation() -> MmapAnnotation<'static> {
    MmapAnnotation::Misc {
        name: "windows-v281-contract",
    }
}

#[test]
fn contract_fixed_mapping_requires_allocation_granularity_for_new_reservation() {
    let _lock = isolate();
    let page = Windows::BYTES_IN_PAGE;
    let allocation = 4 * 1024 * 1024usize;
    let a = annotation();
    let base = Windows::dzmmap_anywhere(allocation, allocation, MmapStrategy::default(), &a)
        .expect("reserve committed region");
    Windows::munmap(base, allocation).expect("release complete reservation");
    let misaligned = base + page;
    assert!(
        Windows::dzmmap(misaligned, page, MmapStrategy::default(), &a).is_err(),
        "a new Windows reservation cannot begin at a merely page-aligned address"
    );
    // The failed fixed allocation must not silently establish the requested mapping.
    assert_eq!(query(misaligned.as_usize()).unwrap().State, MEM_FREE);
}

#[test]
fn contract_partial_release_rejected_even_when_virtualquery_regions_are_split() {
    let _lock = isolate();
    let a = annotation();
    let size = 4 * 1024 * 1024usize;
    let page = Windows::BYTES_IN_PAGE;
    let base = Windows::dzmmap_anywhere(size, size, MmapStrategy::QUARANTINE.replace(false), &a)
        .expect("reserve region");
    // Split VirtualQuery regions by committing only the middle page.
    Windows::dzmmap(base + page, page, MmapStrategy::TEST, &a).expect("commit middle page");
    assert!(Windows::munmap(base, page).is_err());
    assert!(Windows::munmap(base + page, size - page).is_err());
    let first = query(base.as_usize()).expect("first page remains mapped");
    let middle = query((base + page).as_usize()).expect("middle page remains mapped");
    assert_eq!(first.State, MEM_RESERVE);
    assert_eq!(middle.State, MEM_COMMIT);
    assert_eq!(first.AllocationBase, middle.AllocationBase);
    Windows::munmap(base, size).expect("release entire reservation");
}

#[test]
fn contract_decommit_recommit_preserves_neighbor_and_zeroes_page() {
    let _lock = isolate();
    let a = annotation();
    let size = 4 * 1024 * 1024usize;
    let page = Windows::BYTES_IN_PAGE;
    let base = Windows::dzmmap_anywhere(size, size, MmapStrategy::QUARANTINE.replace(false), &a)
        .expect("reserve region");
    Windows::dzmmap(base, 2 * page, MmapStrategy::TEST, &a).expect("commit both pages");
    unsafe {
        std::ptr::write_volatile(base.to_mut_ptr::<u8>(), 0xA5);
        std::ptr::write_volatile((base + page).to_mut_ptr::<u8>(), 0x5A);
    }
    Windows::decommit_metadata_test_range(base, page).expect("decommit one page");
    assert_eq!(query(base.as_usize()).unwrap().State, MEM_RESERVE);
    assert_eq!(query((base + page).as_usize()).unwrap().State, MEM_COMMIT);
    Windows::dzmmap(base, page, MmapStrategy::TEST, &a).expect("recommit page");
    unsafe {
        assert_eq!(std::ptr::read_volatile(base.to_mut_ptr::<u8>()), 0);
        assert_eq!(
            std::ptr::read_volatile((base + page).to_mut_ptr::<u8>()),
            0x5A
        );
    }
    Windows::munmap(base, size).expect("release reservation");
}

#[test]
fn contract_independent_reservations_map_unmap_in_parallel() {
    let _lock = isolate();
    let handles: Vec<_> = (0..6)
        .map(|_| {
            std::thread::spawn(|| {
                let a = annotation();
                let size = 4 * 1024 * 1024usize;
                for _ in 0..48 {
                    let addr = Windows::dzmmap_anywhere(size, size, MmapStrategy::default(), &a)
                        .expect("allocate independent reservation");
                    unsafe { std::ptr::write_volatile(addr.to_mut_ptr::<u8>(), 0x34) };
                    assert_eq!(
                        unsafe { std::ptr::read_volatile(addr.to_mut_ptr::<u8>()) },
                        0x34
                    );
                    Windows::munmap(addr, size).expect("release independent reservation");
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("worker must not panic");
    }
}
