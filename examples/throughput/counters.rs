//! The core's cycle and instruction counters for this thread, through `perf_event_open(2)`,
//! user and kernel both, as the thread CPU clock counts them.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

/// `perf_event_attr` as of `PERF_ATTR_SIZE_VER0`, the fields the kernel has always taken.
#[repr(C)]
struct Attr {
    kind: u32,
    size: u32,
    config: u64,
    sample_period: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    wakeup_events: u32,
    bp_type: u32,
    config1: u64,
}

const TYPE_HARDWARE: u32 = 0;
const CPU_CYCLES: u64 = 0;
const INSTRUCTIONS: u64 = 1;
const FORMAT_TOTAL_TIME_ENABLED: u64 = 1;
const FORMAT_TOTAL_TIME_RUNNING: u64 = 2;
const FORMAT_GROUP: u64 = 8;
const FLAG_DISABLED: u64 = 1;
const FLAG_EXCLUDE_HV: u64 = 1 << 6;
const FLAG_FD_CLOEXEC: libc::c_ulong = 8;
const IOC_ENABLE: libc::c_ulong = 0x2400;
const IOC_DISABLE: libc::c_ulong = 0x2401;
const IOC_RESET: libc::c_ulong = 0x2403;
const IOC_FLAG_GROUP: libc::c_ulong = 1;

/// Cycles and instructions, counted as one group so both cover the same span.
pub struct Counters {
    cycles: OwnedFd,
    _instructions: OwnedFd,
}

#[derive(Clone, Copy, Default)]
pub struct Count {
    pub cycles: u64,
    pub instructions: u64,
}

/// `/proc/sys/kernel/perf_event_paranoid` must be 1 (the user's call, BEN-009).
pub fn check_paranoid() -> Result<(), String> {
    let path = "/proc/sys/kernel/perf_event_paranoid";
    let value = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    match value.trim() {
        "1" => Ok(()),
        other => Err(format!(
            "{path} is {other}, and the throughput bench needs 1: sudo sysctl kernel.perf_event_paranoid=1"
        )),
    }
}

fn open(config: u64, group: Option<&OwnedFd>) -> io::Result<OwnedFd> {
    let attr = Attr {
        kind: TYPE_HARDWARE,
        size: size_of::<Attr>() as u32,
        config,
        sample_period: 0,
        sample_type: 0,
        read_format: FORMAT_GROUP | FORMAT_TOTAL_TIME_ENABLED | FORMAT_TOTAL_TIME_RUNNING,
        flags: if group.is_none() { FLAG_DISABLED } else { 0 } | FLAG_EXCLUDE_HV,
        wakeup_events: 0,
        bp_type: 0,
        config1: 0,
    };
    let leader = group.map_or(-1, AsRawFd::as_raw_fd);
    // SAFETY: `attr` is a valid perf_event_attr of the size it says, alive for the call; pid 0
    // and cpu -1 count this thread on any CPU.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_perf_event_open,
            &raw const attr,
            0 as libc::c_long,
            -1 as libc::c_long,
            libc::c_long::from(leader),
            FLAG_FD_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the kernel just returned this descriptor, and nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}

impl Counters {
    pub fn new() -> io::Result<Self> {
        let cycles = open(CPU_CYCLES, None)?;
        let instructions = open(INSTRUCTIONS, Some(&cycles))?;
        Ok(Self {
            cycles,
            _instructions: instructions,
        })
    }

    fn ioctl(&self, request: libc::c_ulong) {
        // SAFETY: a perf ioctl on the group leader, which this owns.
        let r = unsafe { libc::ioctl(self.cycles.as_raw_fd(), request as _, IOC_FLAG_GROUP) };
        assert_eq!(
            r,
            0,
            "perf ioctl {request:#x}: {}",
            io::Error::last_os_error()
        );
    }

    pub fn start(&self) {
        self.ioctl(IOC_RESET);
        self.ioctl(IOC_ENABLE);
    }

    /// Stop counting and read the group. Panics if the counters were multiplexed, which would
    /// make them estimates.
    pub fn stop(&self) -> Count {
        self.ioctl(IOC_DISABLE);
        // nr, time enabled, time running, cycles, instructions
        let mut v = [0u64; 5];
        // SAFETY: reads at most `v`'s size into it.
        let n = unsafe {
            libc::read(
                self.cycles.as_raw_fd(),
                v.as_mut_ptr().cast(),
                size_of_val(&v),
            )
        };
        assert_eq!(
            n as usize,
            size_of_val(&v),
            "perf read: {}",
            io::Error::last_os_error()
        );
        assert_eq!(v[0], 2, "two counters in the group");
        assert_eq!(v[1], v[2], "the counters were multiplexed");
        Count {
            cycles: v[3],
            instructions: v[4],
        }
    }
}
