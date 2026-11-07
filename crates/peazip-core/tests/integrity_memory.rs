use peazip_core::{Limits, test_archive};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    io::{Cursor, Write},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

struct MeasuredAllocator;
static MEASURE: AtomicBool = AtomicBool::new(false);
static LARGEST: AtomicUsize = AtomicUsize::new(0);

fn record(size: usize) {
    if MEASURE.load(Relaxed) {
        LARGEST.fetch_max(size, Relaxed);
    }
}

unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record(size);
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;

#[test]
fn zip_and_sevenzip_integrity_testing_never_allocate_a_file_sized_output_buffer() {
    let size = 16 * 1024 * 1024;
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "large",
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .unwrap();
    writer.write_all(&vec![0; size]).unwrap();
    let zip = writer.finish().unwrap().into_inner();
    let limits = Limits {
        max_compression_ratio: 100_000,
        ..Limits::default()
    };
    for bytes in [
        zip.as_slice(),
        &include_bytes!("fixtures/7z-large-stream.7z")[..],
    ] {
        LARGEST.store(0, Relaxed);
        MEASURE.store(true, Relaxed);
        let report = test_archive(bytes, limits);
        MEASURE.store(false, Relaxed);
        let report = report.unwrap();
        assert_eq!(report.total_bytes, size as u64);
        assert_eq!(report.files, 1);
        assert_eq!(report.checksummed_files, 1);
        // The 7z dictionary is 1 MiB; retaining the 16 MiB output would exceed this ceiling.
        assert!(
            LARGEST.load(Relaxed) < 4 * 1024 * 1024,
            "largest allocation: {}",
            LARGEST.load(Relaxed)
        );
    }
}
