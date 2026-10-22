use okf_core::{Error, Result};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

/// Cooperative request interruption. The caller supplies a timeout and cancellation signal.
#[derive(Clone, Debug)]
pub struct RequestControl {
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
}

impl RequestControl {
    pub fn new(timeout: Duration, cancelled: Arc<AtomicBool>) -> Self {
        Self {
            deadline: Instant::now() + timeout,
            cancelled,
        }
    }

    pub fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(Error::new("CANCELLED", "request cancelled", 7));
        }
        if Instant::now() >= self.deadline {
            return Err(Error::new(
                "DEADLINE_EXCEEDED",
                "request deadline exceeded",
                7,
            ));
        }
        Ok(())
    }
}

/// One unit per directory opened and per iterator step (including end-of-directory).
pub struct ScanBudget<'a> {
    pub control: &'a RequestControl,
    limit: usize,
    used: usize,
}

impl<'a> ScanBudget<'a> {
    pub fn new(limit: usize, control: &'a RequestControl) -> Self {
        Self {
            control,
            limit,
            used: 0,
        }
    }

    pub fn charge(&mut self) -> Result<()> {
        self.control.check()?;
        if self.used >= self.limit {
            return Err(Error::new(
                "SCAN_LIMIT",
                "scan budget reached; increase --max-scan or narrow the bundle",
                7,
            ));
        }
        self.used += 1;
        Ok(())
    }

    pub fn used(&self) -> usize {
        self.used
    }
}

pub fn interrupted(error: &Error) -> bool {
    matches!(error.code.as_str(), "CANCELLED" | "DEADLINE_EXCEEDED")
}
