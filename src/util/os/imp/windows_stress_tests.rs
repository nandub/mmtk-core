//! Windows-only stress qualification of the native MMTk operating-system backend.
//! These are bounded tests, not a proof of thread safety or leak freedom.
use super::*;

const REGION_BYTES: usize = 4 * 1024 * 1024;

struct Reservation(Address);

impl Reservation {
    fn new() -> Self {
        let annotation = MmapAnnotation::Misc {
            name: "windows-backend-stress",
        };
        let base = Windows::dzmmap_anywhere(
            REGION_BYTES,
            REGION_BYTES,
            MmapStrategy::QUARANTINE.replace(false),
            &annotation,
        )
        .expect("reserve aligned Windows address range");
        Self(base)
    }

    fn base(&self) -> Address {
        self.0
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        Windows::munmap(self.0, REGION_BYTES)
            .expect("release entire Windows reservation in stress test");
    }
}

fn lifecycle(iterations: usize) {
    let annotation = MmapAnnotation::Misc {
        name: "windows-backend-stress",
    };
    let page = Windows::BYTES_IN_PAGE;
    for i in 0..iterations {
        let allocation = Reservation::new();
        let base = allocation.base();
        let first = base + page * (i % 16);
        let neighbor = first + page;
        assert_eq!(
            Windows::dzmmap(first, page * 2, MmapStrategy::TEST, &annotation)
                .expect("commit two pages"),
            first
        );
        unsafe {
            std::ptr::write_volatile(first.to_mut_ptr::<u8>(), 0xA5);
            std::ptr::write_volatile(neighbor.to_mut_ptr::<u8>(), 0x5A);
        }
        Windows::decommit_metadata_test_range(first, page).expect("decommit first page");
        assert_eq!(query(first.as_usize()).unwrap().State, MEM_RESERVE);
        assert_eq!(query(neighbor.as_usize()).unwrap().State, MEM_COMMIT);
        assert_eq!(
            unsafe { std::ptr::read_volatile(neighbor.to_mut_ptr::<u8>()) },
            0x5A
        );
        Windows::dzmmap(first, page, MmapStrategy::TEST, &annotation).expect("recommit first page");
        assert_eq!(
            unsafe { std::ptr::read_volatile(first.to_mut_ptr::<u8>()) },
            0
        );
        // Reservation::drop releases the full allocation, even on assertion failure.
    }
}

#[test]
fn stress_reserve_commit_decommit_recommit_release() {
    let _vm_test_lock = crate::util::raw_memory_freelist::tests::MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    lifecycle(256);
}

#[test]
fn stress_independent_reservations_parallel() {
    let _vm_test_lock = crate::util::raw_memory_freelist::tests::MUTEX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // Concurrent reservations are independent; intentionally does not exercise
    // overlapping-address races or externally synchronized use of one mapping.
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8).map(|_| scope.spawn(|| lifecycle(128))).collect();
        for handle in handles {
            handle.join().expect("Windows memory stress worker");
        }
    });
}
