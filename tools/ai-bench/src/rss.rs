//! Peak memory, for the memory gate.

/// Peak memory in MB: the larger of this process's lifetime-max physical
/// footprint (what Activity Monitor shows; includes Metal/Core ML
/// allocations that plain RSS undercounts) and the peak RSS of its exited
/// child processes (llama-server, the Apple helper). System daemons used by
/// Apple's engines are not visible here; the report says so.
pub fn peak_rss_mb() -> f64 {
    let children = {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: getrusage only writes into the provided struct.
        if unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &mut usage) } == 0 {
            usage.ru_maxrss as f64 / 1_048_576.0 // bytes on macOS
        } else {
            0.0
        }
    };
    let own = {
        let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
        // SAFETY: proc_pid_rusage fills a rusage_info_v4 for our own pid.
        let ok = unsafe {
            libc::proc_pid_rusage(
                std::process::id() as i32,
                libc::RUSAGE_INFO_V4,
                &mut info as *mut _ as *mut libc::rusage_info_t,
            )
        } == 0;
        if ok {
            info.ri_lifetime_max_phys_footprint as f64 / 1_048_576.0
        } else {
            0.0
        }
    };
    own.max(children)
}
