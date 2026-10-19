use crate::{Error, Result};

/// Set native unused-page release delay in milliseconds (`-1` disables timed purging).
/// This is process-wide best-effort tuning, not a memory usage limit. Rust/Standard
/// accept the setting without changing the application's allocator policy.
pub fn config(release_delay_ms: i64) -> Result<()> {
    if release_delay_ms < -1 {
        return Err(Error::Backend("release delay must be -1 or nonnegative"));
    }
    #[cfg(feature = "mimalloc")]
    {
        // ABI value from the fixed mimalloc v3 header; sys exposes the option functions
        // but not this v3 enum constant. The native submodule is pinned in Git.
        const PURGE_DELAY: libmimalloc_sys::mi_option_t = 15;
        let value = std::ffi::c_long::try_from(release_delay_ms)
            .map_err(|_| Error::Backend("release delay exceeds native range"))?;
        // SAFETY: validated option enum and c_long value, no borrowed native buffers.
        unsafe {
            libmimalloc_sys::mi_option_set(PURGE_DELAY, value);
            if libmimalloc_sys::mi_option_get(PURGE_DELAY) != value {
                return Err(Error::Backend("mimalloc rejected release delay"));
            }
        }
    }
    #[cfg(all(feature = "jemalloc", not(feature = "mimalloc")))]
    crate::backend::configure_release_delay(release_delay_ms)?;
    Ok(())
}
