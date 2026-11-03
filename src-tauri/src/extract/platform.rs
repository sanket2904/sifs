//! What reading needs from the operating system: a low priority for the
//! threads that read, and whether a file may be read at all.

use std::fs::Metadata;

/// Why a file in iCloud that isn't downloaded is skipped: reading it would
/// download it.
pub const NOT_DOWNLOADED: &str = "in iCloud, not downloaded";

/// Lowers the calling thread's priority, so reading never slows what the
/// person is doing: macOS's "utility" quality of service, or a nice value
/// of 10 on Linux.
pub fn lower_thread_priority() {
    #[cfg(target_os = "macos")]
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
    #[cfg(target_os = "linux")]
    unsafe {
        // On Linux a thread id names one thread to setpriority.
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 10);
    }
}

/// Whether the file may be opened, or the reason to skip it.
pub fn may_read(meta: &Metadata) -> Result<(), &'static str> {
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        // <sys/stat.h>: the file's data is not on this Mac (an iCloud
        // placeholder); opening it would fetch it.
        const SF_DATALESS: u32 = 0x4000_0000;
        if meta.st_flags() & SF_DATALESS != 0 {
            return Err(NOT_DOWNLOADED);
        }
    }
    let _ = meta;
    Ok(())
}
