//! Linux Landlock and seccomp setup for the tab worker.
//!
//! This code runs at the start of worker mode, before any Rust threads are
//! created. Landlock blocks filesystem writes and, where the kernel supports
//! them, TCP listening, device ioctls, abstract Unix sockets and signals to
//! processes outside the sandbox. A seccomp filter blocks starting programs
//! and processes, debugging or signalling other processes, namespaces, Unix
//! socket connections, terminal input injection and rarely needed kernel
//! interfaces. Reads and outbound network connections remain available.

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
const IOCTL_DEV_ABI_V5: u64 = 1 << 15;
const BIND_TCP_ABI_V4: u64 = 1 << 0;
const SCOPE_ABSTRACT_UNIX_SOCKET_ABI_V6: u64 = 1 << 0;
const SCOPE_SIGNAL_ABI_V6: u64 = 1 << 1;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
    scoped: u64,
}

/// Restrict the calling process, failing closed if Landlock ABI v3 or the
/// seccomp filter is unavailable.
pub(super) fn restrict() -> Result<(), String> {
    restrict_with_landlock()?;
    restrict_system_calls()
}

fn restrict_with_landlock() -> Result<(), String> {
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

    // Newer kernels handle more rights; with no rules granting them, handling
    // a right denies it.
    let attributes = RulesetAttr {
        handled_access_fs: WRITE_RIGHTS_ABI_V3 | if abi >= 5 { IOCTL_DEV_ABI_V5 } else { 0 },
        handled_access_net: if abi >= 4 { BIND_TCP_ABI_V4 } else { 0 },
        scoped: if abi >= 6 {
            SCOPE_ABSTRACT_UNIX_SOCKET_ABI_V6 | SCOPE_SIGNAL_ABI_V6
        } else {
            0
        },
    };
    // Each kernel reads only the fields its ABI defines.
    let attributes_size = match abi {
        3 => size_of::<u64>(),
        4 | 5 => 2 * size_of::<u64>(),
        _ => size_of::<RulesetAttr>(),
    };
    // SAFETY: attributes points to a live, correctly laid out ruleset
    // structure of at least `attributes_size` bytes; zero flags request a
    // regular ruleset descriptor.
    let raw_fd = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &attributes as *const RulesetAttr,
            attributes_size as libc::c_long,
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

    // Landlock and seccomp require no_new_privs for unprivileged restriction.
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

#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH: u32 = 0xC000_003E; // EM_X86_64, 64-bit, little-endian
#[cfg(target_arch = "aarch64")]
const AUDIT_ARCH: u32 = 0xC000_00B7; // EM_AARCH64, 64-bit, little-endian
// System calls of the x32 ABI share the x86_64 architecture value.
#[cfg(target_arch = "x86_64")]
const X32_SYSCALL_BIT: u32 = 0x4000_0000;

/// System calls a tab process never needs, denied with EPERM.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const DENIED: &[libc::c_long] = &[
    libc::SYS_execve,
    libc::SYS_execveat,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_fork,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_vfork,
    libc::SYS_ptrace,
    libc::SYS_process_vm_readv,
    libc::SYS_process_vm_writev,
    libc::SYS_process_madvise,
    libc::SYS_tkill,
    libc::SYS_rt_sigqueueinfo,
    libc::SYS_rt_tgsigqueueinfo,
    libc::SYS_pidfd_open,
    libc::SYS_pidfd_send_signal,
    libc::SYS_pidfd_getfd,
    libc::SYS_unshare,
    libc::SYS_setns,
    libc::SYS_mount,
    libc::SYS_umount2,
    libc::SYS_pivot_root,
    libc::SYS_chroot,
    libc::SYS_open_tree,
    libc::SYS_move_mount,
    libc::SYS_fsopen,
    libc::SYS_fsconfig,
    libc::SYS_fsmount,
    libc::SYS_fspick,
    libc::SYS_mount_setattr,
    libc::SYS_keyctl,
    libc::SYS_add_key,
    libc::SYS_request_key,
    libc::SYS_bpf,
    libc::SYS_perf_event_open,
    libc::SYS_userfaultfd,
    libc::SYS_io_uring_setup,
    libc::SYS_io_uring_enter,
    libc::SYS_io_uring_register,
    libc::SYS_kexec_load,
    libc::SYS_kexec_file_load,
    libc::SYS_init_module,
    libc::SYS_finit_module,
    libc::SYS_delete_module,
];

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const NAMESPACES: u32 = (libc::CLONE_NEWNS
    | libc::CLONE_NEWCGROUP
    | libc::CLONE_NEWUTS
    | libc::CLONE_NEWIPC
    | libc::CLONE_NEWUSER
    | libc::CLONE_NEWPID
    | libc::CLONE_NEWNET) as u32;

// Offsets into struct seccomp_data. Both supported architectures are
// little-endian, so an argument's low 32 bits come first.
const SYSCALL_NUMBER: u32 = 0;
const ARCHITECTURE: u32 = 4;
const fn argument(index: u32) -> u32 {
    16 + 8 * index
}

fn load(offset: u32) -> libc::sock_filter {
    libc::sock_filter {
        code: (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
        jt: 0,
        jf: 0,
        k: offset,
    }
}

/// Compare the loaded value with `value`, skipping `if_true` or `if_false`
/// following instructions.
fn jump(condition: u32, value: u32, if_true: u8, if_false: u8) -> libc::sock_filter {
    libc::sock_filter {
        code: (libc::BPF_JMP | condition | libc::BPF_K) as u16,
        jt: if_true,
        jf: if_false,
        k: value,
    }
}

fn give(action: u32) -> libc::sock_filter {
    libc::sock_filter {
        code: (libc::BPF_RET | libc::BPF_K) as u16,
        jt: 0,
        jf: 0,
        k: action,
    }
}

/// The filter for process `pid`. Each check either returns or leaves the
/// system call number loaded for the next.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn filter(pid: u32) -> Vec<libc::sock_filter> {
    let allow = libc::SECCOMP_RET_ALLOW;
    let deny = libc::SECCOMP_RET_ERRNO | libc::EPERM as u32;
    let mut program = vec![
        load(ARCHITECTURE),
        jump(libc::BPF_JEQ, AUDIT_ARCH, 1, 0),
        give(libc::SECCOMP_RET_KILL_PROCESS),
        load(SYSCALL_NUMBER),
    ];
    #[cfg(target_arch = "x86_64")]
    program.extend([jump(libc::BPF_JGE, X32_SYSCALL_BIT, 0, 1), give(deny)]);
    for &number in DENIED {
        program.extend([jump(libc::BPF_JEQ, number as u32, 0, 1), give(deny)]);
    }
    // A filter cannot read clone3's flags. glibc falls back to clone when
    // clone3 is missing.
    program.extend([
        jump(libc::BPF_JEQ, libc::SYS_clone3 as u32, 0, 1),
        give(libc::SECCOMP_RET_ERRNO | libc::ENOSYS as u32),
    ]);
    // Threads only: no new processes or namespaces.
    program.extend([
        jump(libc::BPF_JEQ, libc::SYS_clone as u32, 0, 5),
        load(argument(0)),
        jump(libc::BPF_JSET, NAMESPACES, 2, 0),
        jump(libc::BPF_JSET, libc::CLONE_THREAD as u32, 0, 1),
        give(allow),
        give(deny),
    ]);
    // Signals only to this process; tgkill also checks that the thread is ours.
    for number in [libc::SYS_kill, libc::SYS_tgkill] {
        program.extend([
            jump(libc::BPF_JEQ, number as u32, 0, 4),
            load(argument(0)),
            jump(libc::BPF_JEQ, pid, 0, 1),
            give(allow),
            give(deny),
        ]);
    }
    // No Unix sockets, which reach agents and session services such as
    // D-Bus, SSH and Docker.
    program.extend([
        jump(libc::BPF_JEQ, libc::SYS_socket as u32, 0, 4),
        load(argument(0)),
        jump(libc::BPF_JEQ, libc::AF_UNIX as u32, 0, 1),
        give(deny),
        give(allow),
    ]);
    // No typing into a terminal or console.
    program.extend([
        jump(libc::BPF_JEQ, libc::SYS_ioctl as u32, 0, 5),
        load(argument(1)),
        jump(libc::BPF_JEQ, libc::TIOCSTI as u32, 1, 0),
        jump(libc::BPF_JEQ, libc::TIOCLINUX as u32, 0, 1),
        give(deny),
        give(allow),
    ]);
    program.push(give(allow));
    program
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn restrict_system_calls() -> Result<(), String> {
    let mut program = filter(std::process::id());
    let program = libc::sock_fprog {
        len: program.len() as libc::c_ushort,
        filter: program.as_mut_ptr(),
    };
    // SAFETY: the program points to `len` live filter instructions for the
    // duration of the call; the kernel copies them. no_new_privs is already
    // set by the Landlock restriction.
    let installed = unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_SET_MODE_FILTER as libc::c_ulong,
            libc::SECCOMP_FILTER_FLAG_TSYNC,
            &program as *const libc::sock_fprog,
        )
    };
    if installed != 0 {
        return Err(last_os_error("seccomp filter"));
    }
    Ok(())
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn restrict_system_calls() -> Result<(), String> {
    Err("the tab system call filter supports x86_64 and aarch64".into())
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

#[cfg(all(test, any(target_arch = "x86_64", target_arch = "aarch64")))]
mod tests {
    use super::*;

    /// Run the classic BPF subset the filter uses over one system call.
    fn verdict(arch: u32, number: libc::c_long, arguments: [u64; 6]) -> u32 {
        let program = filter(1234);
        let mut data = vec![0_u8; 64];
        data[0..4].copy_from_slice(&(number as u32).to_le_bytes());
        data[4..8].copy_from_slice(&arch.to_le_bytes());
        for (index, value) in arguments.iter().enumerate() {
            data[16 + 8 * index..24 + 8 * index].copy_from_slice(&value.to_le_bytes());
        }
        let mut accumulator = 0_u32;
        let mut counter = 0;
        loop {
            let instruction = program[counter];
            let code = u32::from(instruction.code);
            counter += 1;
            match code {
                c if c == libc::BPF_LD | libc::BPF_W | libc::BPF_ABS => {
                    let at = instruction.k as usize;
                    accumulator = u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
                }
                c if c == libc::BPF_RET | libc::BPF_K => return instruction.k,
                c => {
                    let taken = match c & !libc::BPF_JMP {
                        x if x == libc::BPF_JEQ => accumulator == instruction.k,
                        x if x == libc::BPF_JGE => accumulator >= instruction.k,
                        x if x == libc::BPF_JSET => accumulator & instruction.k != 0,
                        _ => panic!("unexpected instruction {code:#x}"),
                    };
                    counter += usize::from(if taken {
                        instruction.jt
                    } else {
                        instruction.jf
                    });
                }
            }
        }
    }

    fn call(number: libc::c_long, arguments: [u64; 6]) -> u32 {
        verdict(AUDIT_ARCH, number, arguments)
    }

    #[test]
    fn the_filter_allows_threads_and_web_requests_but_not_escapes() {
        let allow = libc::SECCOMP_RET_ALLOW;
        let deny = libc::SECCOMP_RET_ERRNO | libc::EPERM as u32;
        let thread = (libc::CLONE_VM | libc::CLONE_THREAD | libc::CLONE_SIGHAND) as u64;
        assert_eq!(
            verdict(0x4000_0003, libc::SYS_read, [0; 6]),
            libc::SECCOMP_RET_KILL_PROCESS
        );
        assert_eq!(call(libc::SYS_read, [0; 6]), allow);
        assert_eq!(call(libc::SYS_execve, [0; 6]), deny);
        assert_eq!(call(libc::SYS_ptrace, [0; 6]), deny);
        assert_eq!(call(libc::SYS_unshare, [0; 6]), deny);
        assert_eq!(call(libc::SYS_clone, [thread, 0, 0, 0, 0, 0]), allow);
        assert_eq!(
            call(libc::SYS_clone, [libc::SIGCHLD as u64, 0, 0, 0, 0, 0]),
            deny
        );
        let namespace = thread | libc::CLONE_NEWNET as u64;
        assert_eq!(call(libc::SYS_clone, [namespace, 0, 0, 0, 0, 0]), deny);
        assert_eq!(
            call(libc::SYS_clone3, [0; 6]),
            libc::SECCOMP_RET_ERRNO | libc::ENOSYS as u32
        );
        assert_eq!(call(libc::SYS_kill, [1234, 0, 0, 0, 0, 0]), allow);
        assert_eq!(call(libc::SYS_kill, [1, 9, 0, 0, 0, 0]), deny);
        assert_eq!(call(libc::SYS_tgkill, [1234, 1235, 6, 0, 0, 0]), allow);
        assert_eq!(call(libc::SYS_tgkill, [999, 999, 9, 0, 0, 0]), deny);
        let inet = libc::AF_INET as u64;
        assert_eq!(call(libc::SYS_socket, [inet, 1, 0, 0, 0, 0]), allow);
        let unix = libc::AF_UNIX as u64;
        assert_eq!(call(libc::SYS_socket, [unix, 1, 0, 0, 0, 0]), deny);
        let inject = libc::TIOCSTI as u64;
        assert_eq!(call(libc::SYS_ioctl, [0, inject, 0, 0, 0, 0]), deny);
        let console = libc::TIOCLINUX as u64;
        assert_eq!(call(libc::SYS_ioctl, [0, console, 0, 0, 0, 0]), deny);
        let readable = libc::FIONREAD as u64;
        assert_eq!(call(libc::SYS_ioctl, [0, readable, 0, 0, 0, 0]), allow);
        #[cfg(target_arch = "x86_64")]
        {
            assert_eq!(call(libc::SYS_fork, [0; 6]), deny);
            let x32 = X32_SYSCALL_BIT as libc::c_long | libc::SYS_read;
            assert_eq!(call(x32, [0; 6]), deny);
        }
    }
}
