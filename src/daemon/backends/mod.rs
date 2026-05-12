pub(crate) mod wayland;

pub(crate) use wayland::*;

use std::os::unix::io::{AsRawFd, RawFd};

#[derive(Clone, Copy, Debug)]
pub(crate) struct RawFdWatcher {
    fd: RawFd,
}

impl RawFdWatcher {
    pub(crate) fn new(fd: RawFd) -> Self {
        Self { fd }
    }
}

impl AsRawFd for RawFdWatcher {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}
