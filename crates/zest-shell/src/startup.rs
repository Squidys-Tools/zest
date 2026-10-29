//! Launch-at-startup (opt-in): writes HKCU `...\Run\Zest` (no admin needed).

use anyhow::Result;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Zest";

/// Write or remove the Run-key entry so Zest starts at sign-in. Idempotent:
/// enabling rewrites the current exe path, disabling a missing value is a
/// no-op. Called on launch and after the settings window closes.
pub fn set_enabled(enabled: bool) -> Result<()> {
    set_enabled_in(RUN_KEY, RUN_VALUE, enabled)
}

/// Whether a Run value for Zest is present. Used to sanity-check writes.
pub fn is_enabled() -> bool {
    registry::get_string(RUN_KEY, RUN_VALUE)
        .ok()
        .flatten()
        .is_some()
}

/// The `enabled`/`value` parts of [`set_enabled`], parameterised so tests can
/// aim at a throwaway key instead of the real Run key.
fn set_enabled_in(subkey: &str, value_name: &str, enabled: bool) -> Result<()> {
    if enabled {
        registry::set_string(subkey, value_name, &run_command()?)
    } else {
        registry::delete_value(subkey, value_name)
    }
}

/// What the Run key launches: the running exe, quoted for paths with spaces.
fn run_command() -> Result<String> {
    let exe = std::env::current_exe()?;
    Ok(format!("\"{}\"", exe.display()))
}

#[cfg(windows)]
mod registry {
    use anyhow::{Context, Result};
    use windows::core::HSTRING;
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ,
        RRF_RT_REG_SZ,
    };

    /// An open HKEY that closes on drop.
    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }

    fn open_for_write(subkey: &str) -> Result<Key> {
        let subkey = HSTRING::from(subkey);
        let mut raw = HKEY::default();
        unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                &subkey,
                None,
                windows::core::PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut raw,
                None,
            )
            .ok()
            .with_context(|| format!("RegCreateKeyExW HKCU\\{subkey}"))?;
        }
        Ok(Key(raw))
    }

    /// `Ok(None)` when the key itself is missing.
    fn open_with(subkey: &str, sam: REG_SAM_FLAGS) -> Result<Option<Key>> {
        let subkey = HSTRING::from(subkey);
        let mut raw = HKEY::default();
        let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &subkey, None, sam, &mut raw) };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        status
            .ok()
            .with_context(|| format!("RegOpenKeyExW HKCU\\{subkey}"))?;
        Ok(Some(Key(raw)))
    }

    /// Write a REG_SZ value (creating the subkey if needed).
    pub fn set_string(subkey: &str, value: &str, data: &str) -> Result<()> {
        let key = open_for_write(subkey)?;
        let name = HSTRING::from(value);
        // REG_SZ is UTF-16 and must include the terminating NUL.
        let mut wide: Vec<u16> = data.encode_utf16().collect();
        wide.push(0);
        let bytes =
            unsafe { std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2) };
        unsafe {
            RegSetValueExW(key.0, &name, None, REG_SZ, Some(bytes))
                .ok()
                .with_context(|| format!("RegSetValueExW {subkey}\\{value}"))?;
        }
        Ok(())
    }

    /// Read a REG_SZ value; `Ok(None)` when the key or value is absent.
    pub fn get_string(subkey: &str, value: &str) -> Result<Option<String>> {
        let subkey_h = HSTRING::from(subkey);
        let value_h = HSTRING::from(value);
        unsafe {
            // Two-call pattern: size first, then the buffer. RegGetValueW opens
            // the key itself, so no open handle is needed.
            let mut size: u32 = 0;
            let status = RegGetValueW(
                HKEY_CURRENT_USER,
                &subkey_h,
                &value_h,
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            );
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            status
                .ok()
                .with_context(|| format!("RegGetValueW {subkey}\\{value}"))?;
            if size == 0 {
                return Ok(None);
            }
            let mut buf = vec![0u16; (size as usize).div_ceil(2)];
            RegGetValueW(
                HKEY_CURRENT_USER,
                &subkey_h,
                &value_h,
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            )
            .ok()
            .with_context(|| format!("RegGetValueW {subkey}\\{value}"))?;
            while buf.last() == Some(&0) {
                buf.pop();
            }
            Ok(Some(String::from_utf16_lossy(&buf)))
        }
    }

    /// Delete a value. Missing keys and values are not errors — the goal is
    /// the value's absence, however it got there.
    pub fn delete_value(subkey: &str, value: &str) -> Result<()> {
        // Deleting needs KEY_SET_VALUE; a read-only handle gets access denied.
        let Some(key) = open_with(subkey, KEY_SET_VALUE)? else {
            return Ok(());
        };
        let name = HSTRING::from(value);
        let status = unsafe { RegDeleteValueW(key.0, &name) };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        status
            .ok()
            .with_context(|| format!("RegDeleteValueW {subkey}\\{value}"))
    }

    /// Remove a whole subkey (test cleanup only — values-only deletes are the
    /// normal path).
    #[cfg(test)]
    pub fn delete_key(subkey: &str) -> Result<()> {
        let subkey_h = HSTRING::from(subkey);
        let status = unsafe {
            windows::Win32::System::Registry::RegDeleteKeyW(HKEY_CURRENT_USER, &subkey_h)
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        status
            .ok()
            .with_context(|| format!("RegDeleteKeyW HKCU\\{subkey}"))
    }
}

#[cfg(not(windows))]
mod registry {
    use anyhow::Result;

    pub fn set_string(_subkey: &str, _value: &str, _data: &str) -> Result<()> {
        Ok(())
    }

    pub fn get_string(_subkey: &str, _value: &str) -> Result<Option<String>> {
        Ok(None)
    }

    pub fn delete_value(_subkey: &str, _value: &str) -> Result<()> {
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// A throwaway key under HKCU, never the real Run key.
    fn test_subkey(name: &str) -> String {
        format!("Software\\Zest\\test-{}-{name}", std::process::id())
    }

    #[test]
    fn set_get_delete_round_trip() {
        let key = test_subkey("round-trip");
        registry::delete_key(&key).ok(); // clean slate, ignore leftovers

        assert_eq!(registry::get_string(&key, "Run").unwrap(), None);

        registry::set_string(&key, "Run", "\"zest.exe\" --quiet").expect("write should succeed");
        assert_eq!(
            registry::get_string(&key, "Run").unwrap().as_deref(),
            Some("\"zest.exe\" --quiet")
        );

        registry::delete_value(&key, "Run").expect("delete should succeed");
        assert_eq!(registry::get_string(&key, "Run").unwrap(), None);

        // Deleting a missing value is a no-op, not an error.
        registry::delete_value(&key, "Run").unwrap();

        registry::delete_key(&key).unwrap();
    }

    #[test]
    fn set_enabled_in_writes_and_removes() {
        let key = test_subkey("enabled");
        registry::delete_key(&key).ok();

        set_enabled_in(&key, "Run", true).expect("enable should succeed");
        let written = registry::get_string(&key, "Run").unwrap();
        assert!(written.is_some(), "enable should leave a Run value");
        assert!(
            written.unwrap().starts_with('"'),
            "the command is a quoted exe path"
        );

        set_enabled_in(&key, "Run", false).expect("disable should succeed");
        assert_eq!(registry::get_string(&key, "Run").unwrap(), None);

        registry::delete_key(&key).unwrap();
    }
}
