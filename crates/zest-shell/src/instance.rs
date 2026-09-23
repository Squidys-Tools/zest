//! Single-instance guard: a named mutex held for the process lifetime so a
//! second launch no-ops instead of double-registering the hotkey (SQU-26).

use anyhow::{Context, Result};

/// Session-local mutex name (`Local\` = one instance per login session).
#[cfg(windows)]
const MUTEX_NAME: windows::core::PCWSTR = windows::core::w!("Local\\ZestSingleInstance");

/// Keeps the single-instance mutex alive; dropping it releases the mutex.
#[cfg(windows)]
pub struct InstanceGuard(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;
        // Best-effort; the OS also closes the handle on process exit.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Acquire the single-instance mutex.
///
/// Returns `Ok(None)` when another live Zest process already owns the mutex;
/// the caller should exit without bringing up tray or hotkey. Returns
/// `Ok(Some(guard))` for the first instance — hold the guard for as long as
/// the app should stay "the" instance.
#[cfg(windows)]
pub fn acquire() -> Result<Option<InstanceGuard>> {
    use windows::Win32::Foundation::{
        GetLastError, SetLastError, ERROR_ALREADY_EXISTS, ERROR_SUCCESS,
    };
    use windows::Win32::System::Threading::CreateMutexW;

    unsafe {
        // Make sure ERROR_ALREADY_EXISTS below is set by CreateMutexW itself.
        SetLastError(ERROR_SUCCESS);
        let handle = CreateMutexW(None, true, MUTEX_NAME).context("CreateMutexW")?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
            Ok(None)
        } else {
            Ok(Some(InstanceGuard(handle)))
        }
    }
}

/// Non-Windows builds have no tray/hotkey to guard; always first instance.
#[cfg(not(windows))]
pub struct InstanceGuard;

#[cfg(not(windows))]
pub fn acquire() -> Result<Option<InstanceGuard>> {
    Ok(Some(InstanceGuard))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    // One test so the process-wide mutex is not raced by parallel tests.
    #[test]
    fn second_acquire_no_ops_until_guard_drops() {
        let first = acquire().expect("acquire");
        assert!(first.is_some(), "first acquire should own the mutex");

        let second = acquire().expect("acquire while held");
        assert!(
            second.is_none(),
            "second acquire must see the live instance"
        );

        drop(first);
        let again = acquire().expect("reacquire after drop");
        assert!(again.is_some(), "dropped guard must release the mutex");
    }
}
