//! Minimal Linux Landlock setup for the tab worker.
//!
//! This code runs at the start of worker mode, before any Rust threads are
//! created. It handles only filesystem write rights; reads, execution,
//! networking, and process creation remain available.

use std::{io, mem::size_of, ptr::null};

const LANDLOCK_CREATE_RULESET_VERSION: libc::c_long = 1;
const LANDLOCK_RESTRICT_SELF_TSYNC: libc::c_long = 1;
const REQUIRED_ABI: libc::c_long = 3;

// Landlock filesystem rights available through ABI v3. Read and execute bits
// are deliberately absent, preserving local browsing and dynamic library use.
const WRITE_RIGHTS_ABI_V3: u64 = (1 << 1) // WRITE_FILE
    | (1 << 4) // REMOVE_DIR
    | (1 << 5) // REMOVE_FILE
    | (1 << 6) // MAKE_CHAR
    | (1 << 7) // MAKE_DIR
    | (1 << 8) // MAKE_REG
    | (1 << 9) // MAKE_SOCK
    | (1 << 10) // MAKE_FIFO
    | (1 << 11) // MAKE_BLOCK
    | (1 << 12) // MAKE_SYM
    | (1 << 13) // REFER
    | (1 << 14); // TRUNCATE

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
}

pub(super) fn restrict_writes() -> Result<(), String> {
    // Query the kernel ABI first. The VERSION flag ignores the attribute
    // pointer and returns the supported ABI number.
    // SAFETY: the VERSION query accepts a null attribute pointer and zero size.
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            null::<RulesetAttr>(),
            0 as libc::c_long,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    if abi < REQUIRED_ABI {
        if abi < 0 {
            return Err(last_os_error("Landlock ABI query"));
        }
        return Err(format!(
            "Landlock ABI {REQUIRED_ABI} is required to block file writes and truncation (kernel reports ABI {abi})"
        ));
    }

    let attributes = RulesetAttr {
        handled_access_fs: WRITE_RIGHTS_ABI_V3,
    };
    // SAFETY: attributes points to a live, correctly laid out v1 ruleset
    // structure; zero flags request a regular ruleset descriptor.
    let raw_fd = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &attributes as *const RulesetAttr,
            size_of::<RulesetAttr>() as libc::c_long,
            0 as libc::c_long,
        )
    };
    if raw_fd < 0 {
        return Err(last_os_error("Landlock ruleset creation"));
    }
    if raw_fd > libc::c_int::MAX as libc::c_long {
        // A valid process file descriptor always fits in c_int. Close the
        // unexpected value rather than allowing a truncated descriptor.
        // SAFETY: raw_fd is the descriptor returned by the successful syscall.
        unsafe { libc::close(raw_fd as libc::c_int) };
        return Err("Landlock returned an invalid ruleset descriptor".into());
    }
    let ruleset = RulesetFd(raw_fd as libc::c_int);

    // Landlock requires no_new_privs for unprivileged restriction.
    // SAFETY: prctl receives the documented scalar arguments and no pointers.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(last_os_error("PR_SET_NO_NEW_PRIVS"));
    }

    // ABI v8 adds atomic all-thread restriction. On earlier kernels, this is
    // called before worker startup creates any sibling threads.
    let flags = if abi >= 8 {
        LANDLOCK_RESTRICT_SELF_TSYNC
    } else {
        0
    };
    // SAFETY: the descriptor is a live Landlock ruleset; flags is either zero
    // or the ABI v8 all-thread synchronization flag.
    let restricted = unsafe {
        libc::syscall(
            libc::SYS_landlock_restrict_self,
            ruleset.0 as libc::c_long,
            flags,
        )
    };
    if restricted != 0 {
        return Err(last_os_error("Landlock restriction"));
    }
    Ok(())
}

fn last_os_error(action: &str) -> String {
    format!("{action} failed: {}", io::Error::last_os_error())
}

struct RulesetFd(libc::c_int);

impl Drop for RulesetFd {
    fn drop(&mut self) {
        // SAFETY: this type owns the descriptor returned by Landlock and closes
        // it exactly once after the restriction has been installed.
        unsafe { libc::close(self.0) };
    }
}
