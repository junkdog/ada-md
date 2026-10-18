//! Panics from code ada doesn't own, caught where ada has something else to show (F-05). A
//! caught panic is quiet: the panic hooks stay silent for it, and the TUI's hook doesn't give the
//! terminal back.

use std::cell::Cell;
use std::panic::{self, UnwindSafe};
use std::sync::Once;

thread_local! {
    static CATCHING: Cell<bool> = const { Cell::new(false) };
}

/// Run `f`, or `None` if it panics.
pub fn catch<T>(f: impl FnOnce() -> T + UnwindSafe) -> Option<T> {
    static QUIET: Once = Once::new();
    QUIET.call_once(|| {
        let hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if !catching() {
                hook(info);
            }
        }));
    });
    let outer = CATCHING.replace(true);
    let result = panic::catch_unwind(f);
    CATCHING.set(outer);
    result.ok()
}

/// Whether this thread is inside [`catch`]: a panic hook installed after its first call checks
/// this itself.
pub fn catching() -> bool {
    CATCHING.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_is_caught_and_the_flag_restored() {
        assert_eq!(catch(|| 7), Some(7));
        assert_eq!(catch(|| -> u8 { panic!("upstream") }), None);
        assert!(!catching());
        // nested: the inner catch leaves the outer one's flag set
        assert_eq!(catch(|| (catch(|| 1), catching())), Some((Some(1), true)));
    }
}
