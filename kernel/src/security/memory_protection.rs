use crate::exec::process::{MemorySegment, SegmentFlags};

/// Enforce W^X policy on a set of memory segments.
/// Returns the number of segments adjusted.
pub fn enforce_wx(segments: &mut [MemorySegment]) -> usize {
    let mut adjusted = 0;

    for segment in segments.iter_mut() {
        let has_write = segment.flags.contains(SegmentFlags::WRITE);
        let has_exec = segment.flags.contains(SegmentFlags::EXECUTE);

        if has_write && has_exec {
            // W^X violation: if it's writable, remove execute
            segment.flags.remove(SegmentFlags::EXECUTE);
            adjusted += 1;
            crate::serial_println!(
                "[W^X] Segment at 0x{:X} size 0x{:X}: removed EXECUTE (was W+X)",
                segment.vaddr,
                segment.size
            );
        }
    }

    adjusted
}

/// Log W^X status for each segment type to serial.
pub fn log_segment_protections(segments: &[MemorySegment]) {
    for seg in segments {
        let r = if seg.flags.contains(SegmentFlags::READ) { "R" } else { "-" };
        let w = if seg.flags.contains(SegmentFlags::WRITE) { "W" } else { "-" };
        let x = if seg.flags.contains(SegmentFlags::EXECUTE) { "X" } else { "-" };
        crate::serial_println!(
            "[W^X] 0x{:016X} size 0x{:X} {}{}{}",
            seg.vaddr,
            seg.size,
            r,
            w,
            x
        );
    }
}

/// Check if W^X is satisfied for all segments.
pub fn verify_wx(segments: &[MemorySegment]) -> bool {
    segments.iter().all(|seg| {
        !(seg.flags.contains(SegmentFlags::WRITE) && seg.flags.contains(SegmentFlags::EXECUTE))
    })
}
