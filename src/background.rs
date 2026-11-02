//! How much background work may get in the way of whatever else the computer is doing.

use std::{
    cell::Cell,
    sync::atomic::{AtomicBool, Ordering},
};

use thread_priority::{ThreadPriority, ThreadPriorityValue, set_current_thread_priority};

/// Whether background work gives way to other programs, see `set_low_priority`.
static LOW_PRIORITY: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// what this thread was last set to
    static LOW: Cell<bool> = const { Cell::new(false) };
}

/// Lets searching and generating give way to all other programs, or have the same priority as
/// them. Low still uses every core that nothing else wants, normal gets done faster when other
/// programs are busy too. Threads take it on at their next piece of work.
pub fn set_low_priority(low: bool) {
    LOW_PRIORITY.store(low, Ordering::Relaxed);
}

pub fn low_priority() -> bool {
    LOW_PRIORITY.load(Ordering::Relaxed)
}

/// Gives this thread the priority that was asked for, if it doesn't have it yet. Workers call it
/// before each piece of work.
pub(crate) fn keep_priority() {
    let low = low_priority();
    if LOW.get() == low {
        return;
    }
    LOW.set(low);
    let priority = if low {
        ThreadPriority::Min
    } else {
        // the middle of the cross platform scale, what threads start with
        ThreadPriority::Crossplatform(ThreadPriorityValue::try_from(50u8).expect("50 is a priority"))
    };
    if let Err(err) = set_current_thread_priority(priority) {
        log::warn!("changing the priority of a worker: {err:?}");
    }
}
