//! How much of the machine cubox may use, set by the Performance menu.

/// 128x128 tiles are 64 KiB as texture plus 96 KiB of estimates kept for downsampling.
const TILES_PER_MIB: usize = 6;

/// The most threads the slider goes to: all cores.
#[cfg(not(target_arch = "wasm32"))]
pub fn max_threads() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get())
}

/// The most threads the slider goes to: all cores. Every worker has its own copy of the world.
#[cfg(target_arch = "wasm32")]
pub fn max_threads() -> usize {
    let cores = web_sys::window().map_or(4.0, |w| w.navigator().hardware_concurrency()) as usize;
    cores.clamp(1, 32)
}

/// Threads at the start: all of them, except for the memory of a phone.
pub fn default_threads() -> usize {
    #[cfg(target_arch = "wasm32")]
    if web_sys::window().is_some_and(|w| w.navigator().max_touch_points() > 0) {
        return max_threads().min(4);
    }
    max_threads()
}

/// The range of the memory slider in MiB. A page can't use much more than this.
pub const CACHE_MIB: (u32, u32) = if cfg!(target_arch = "wasm32") {
    (32, 2048)
} else {
    (64, 16384)
};

/// Where the memory slider starts, in MiB: phones and tablets can't spare that much.
pub fn default_cache_mib() -> u32 {
    #[cfg(target_arch = "wasm32")]
    if web_sys::window().is_some_and(|w| w.navigator().max_touch_points() > 0) {
        return 128;
    }
    1024
}

pub fn tiles_in_mib(mib: u32) -> usize {
    mib as usize * TILES_PER_MIB
}

/// How threads are divided: `(for tiles, for structures and search)`, never more than `threads`
/// together while both have work, and one gets all of them when the other has none.
pub fn share(threads: usize, tiles_busy: bool, finder_busy: bool) -> (usize, usize) {
    match (tiles_busy, finder_busy) {
        (true, true) => (threads - threads / 2, threads / 2),
        _ => (threads, threads),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threads_are_shared_without_going_over() {
        for threads in 1..=32 {
            let (tiles, finder) = share(threads, true, true);
            assert_eq!(tiles + finder, threads);
            assert!(tiles >= 1);
            assert_eq!(share(threads, true, false).0, threads);
            assert_eq!(share(threads, false, true).1, threads);
        }
    }
}
