use crate::{Error, Result};

/// Optional requested-byte accounting. Zero-size allocations are not counted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum StatisticsMode {
    /// No allocation records or counter updates.
    #[default]
    Disabled,
    /// Count successful operations and requested live/peak bytes.
    Basic,
}

/// Per-resource accounting, excluding allocator metadata and reserved pages.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Statistics {
    /// Successful nonzero allocations, excluding reallocations.
    pub allocations: u64,
    /// Individual deallocations, excluding reallocations.
    pub deallocations: u64,
    /// Successful nonzero-to-nonzero reallocations.
    pub reallocations: u64,
    /// Requested bytes in tracked live allocations.
    pub live_bytes: usize,
    /// Highest requested live-byte count since reset.
    pub peak_live_bytes: usize,
}

#[cfg(feature = "statistics")]
#[derive(Default)]
struct State {
    values: Statistics,
    live: std::collections::BTreeMap<usize, usize>,
}

pub(crate) struct Tracker {
    enabled: bool,
    #[cfg(feature = "statistics")]
    state: std::sync::Mutex<State>,
}

impl Tracker {
    pub const fn new() -> Self {
        Self {
            enabled: false,
            #[cfg(feature = "statistics")]
            state: std::sync::Mutex::new(State {
                values: Statistics {
                    allocations: 0,
                    deallocations: 0,
                    reallocations: 0,
                    live_bytes: 0,
                    peak_live_bytes: 0,
                },
                live: std::collections::BTreeMap::new(),
            }),
        }
    }

    #[cfg(feature = "statistics")]
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn mode(&self) -> StatisticsMode {
        if self.enabled {
            StatisticsMode::Basic
        } else {
            StatisticsMode::Disabled
        }
    }

    pub fn set_mode(&mut self, mode: StatisticsMode) -> Result<()> {
        if mode == StatisticsMode::Basic && !cfg!(feature = "statistics") {
            return Err(Error::StatisticsDisabled);
        }
        #[cfg(feature = "statistics")]
        if !self.lock().live.is_empty() {
            return Err(Error::Busy);
        }
        self.enabled = mode == StatisticsMode::Basic;
        Ok(())
    }

    #[inline]
    pub fn allocated(&self, pointer: *mut u8, bytes: usize) {
        #[cfg(feature = "statistics")]
        if self.enabled && bytes != 0 {
            let mut state = self.lock();
            state.live.insert(pointer.addr(), bytes);
            state.values.allocations = state.values.allocations.saturating_add(1);
            state.values.live_bytes += bytes;
            state.values.peak_live_bytes =
                state.values.peak_live_bytes.max(state.values.live_bytes);
        }
        #[cfg(not(feature = "statistics"))]
        let _ = (pointer, bytes);
    }

    #[inline]
    pub fn deallocated(&self, pointer: *mut u8, bytes: usize) {
        #[cfg(feature = "statistics")]
        if self.enabled && bytes != 0 {
            let mut state = self.lock();
            if let Some(size) = state.live.remove(&pointer.addr()) {
                state.values.live_bytes -= size;
                state.values.deallocations = state.values.deallocations.saturating_add(1);
            }
        }
        #[cfg(not(feature = "statistics"))]
        let _ = (pointer, bytes);
    }

    #[inline]
    pub fn resized(&self, old: *mut u8, new: *mut u8, bytes: usize) {
        #[cfg(feature = "statistics")]
        if self.enabled {
            let mut state = self.lock();
            if let Some(previous) = state.live.remove(&old.addr()) {
                state.live.insert(new.addr(), bytes);
                state.values.reallocations = state.values.reallocations.saturating_add(1);
                state.values.live_bytes = state.values.live_bytes - previous + bytes;
                state.values.peak_live_bytes =
                    state.values.peak_live_bytes.max(state.values.live_bytes);
            }
        }
        #[cfg(not(feature = "statistics"))]
        let _ = (old, new, bytes);
    }

    pub fn snapshot(&self) -> Statistics {
        #[cfg(feature = "statistics")]
        if self.enabled {
            return self.lock().values;
        }
        Statistics::default()
    }

    pub fn reset(&mut self) -> Result<()> {
        #[cfg(feature = "statistics")]
        {
            let mut state = self.lock();
            if !state.live.is_empty() {
                return Err(Error::Busy);
            }
            state.values = Statistics::default();
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        #[cfg(feature = "statistics")]
        {
            *self.lock() = State::default();
        }
    }

    pub fn rewind(&mut self, cutoff: usize) {
        #[cfg(feature = "statistics")]
        if self.enabled {
            let mut state = self.lock();
            let discarded = state.live.split_off(&cutoff);
            let mut crossing_bytes = 0;
            let mut crossing_count = 0;
            state.live.retain(|&address, &mut bytes| {
                if bytes > cutoff - address {
                    crossing_bytes += bytes;
                    crossing_count += 1;
                    false
                } else {
                    true
                }
            });
            state.values.deallocations = state
                .values
                .deallocations
                .saturating_add(discarded.len() as u64 + crossing_count);
            state.values.live_bytes -= discarded.values().sum::<usize>() + crossing_bytes;
        }
        #[cfg(not(feature = "statistics"))]
        let _ = cutoff;
    }
}
