//! Windows launcher for write-restricted tab workers.
//!
//! The small Win32 boundary is kept in this module. The parent starts this
//! executable as a broker; this function starts a second copy with a
//! write-restricted token, preserving the inherited worker pipes.

use std::{
    ffi::OsStr,
    io,
    mem::size_of,
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr::{null, null_mut},
};

use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree},
    Security::Authorization::ConvertStringSidToSidW,
    Security::{
        CreateRestrictedToken, DISABLE_MAX_PRIVILEGE, LUA_TOKEN, SID_AND_ATTRIBUTES,
        TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_QUERY, WRITE_RESTRICTED,
    },
    System::{
        Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS,
            JOB_OBJECT_UILIMIT_EXITWINDOWS, JOB_OBJECT_UILIMIT_GLOBALATOMS,
            JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
            JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
            JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateProcessAsUserW, GetCurrentProcess,
            GetExitCodeProcess, INFINITE, OpenProcessToken, PROCESS_INFORMATION, ResumeThread,
            STARTF_USESTDHANDLES, STARTUPINFOW, TerminateProcess, WAIT_OBJECT_0,
            WaitForSingleObject,
        },
    },
};

pub(super) fn launch_worker() -> Result<u32, String> {
    validate_launcher_arguments()?;
    launch_restricted_worker().map_err(|error| error.to_string())
}

fn validate_launcher_arguments() -> Result<(), String> {
    let mut args = std::env::args_os();
    let _executable = args.next();
    if args.next().as_deref() != Some(OsStr::new(super::WINDOWS_LAUNCHER_ARG))
        || args.next().as_deref() != Some(OsStr::new(super::WORKER_ARG))
        || args.next().is_some()
    {
        return Err("invalid restricted-worker launcher arguments".into());
    }
    Ok(())
}

fn launch_restricted_worker() -> io::Result<u32> {
    let executable = std::env::current_exe()?;
    let current_directory = executable
        .parent()
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| Path::new("C:\\Windows\\System32"));
    let executable_wide = wide_nul(executable.as_os_str());
    let current_directory_wide = wide_nul(current_directory.as_os_str());
    let mut command_line = worker_command_line(executable.as_os_str());

    // The inherited standard handles are the private protocol pipes created by
    // Worker::spawn_command_at. Keep them on the child so the broker can wait
    // for its lifetime while the parent communicates directly with the worker.
    let mut startup = STARTUPINFOW::default();
    startup.cb = size_of::<STARTUPINFOW>() as u32;
    startup.dwFlags = STARTF_USESTDHANDLES;
    startup.hStdInput = std_handle(STD_INPUT_HANDLE)?;
    startup.hStdOutput = std_handle(STD_OUTPUT_HANDLE)?;
    startup.hStdError = std_handle(STD_ERROR_HANDLE)?;

    let process_token = open_current_process_token()?;
    let restricted_sid = restricted_code_sid()?;
    let restricted_sid_entry = SID_AND_ATTRIBUTES {
        Sid: restricted_sid.0,
        Attributes: 0,
    };
    let restricted_token = create_write_restricted_token(process_token.0, &restricted_sid_entry)?;
    let job = create_kill_on_close_job()?;

    let mut process_info = PROCESS_INFORMATION::default();
    // SAFETY: all pointers refer to live NUL-terminated buffers or initialized
    // Win32 structures for the duration of the call. The token is a primary
    // token derived from this process; only the three protocol stdio handles
    // are marked for inheritance by the parent Rust process.
    let created = unsafe {
        CreateProcessAsUserW(
            restricted_token.0,
            executable_wide.as_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_SUSPENDED | CREATE_NO_WINDOW,
            null(),
            current_directory_wide.as_ptr(),
            &startup,
            &mut process_info,
        )
    };
    if created == 0 {
        return Err(last_os_error("CreateProcessAsUserW"));
    }
    let process = OwnedHandle::new(process_info.hProcess, "worker process")?;
    let thread = OwnedHandle::new(process_info.hThread, "worker thread")?;

    // Assign before resuming so the worker and any descendants are always
    // covered by the broker's kill-on-close job.
    // SAFETY: both handles are valid handles returned by successful Win32
    // calls, and the job is configured before the process is resumed.
    if unsafe { AssignProcessToJobObject(job.0, process.0) } == 0 {
        // SAFETY: the process handle is valid and the suspended process must
        // not be left running outside the job if assignment fails.
        unsafe { TerminateProcess(process.0, 1) };
        return Err(last_os_error("AssignProcessToJobObject"));
    }

    // SAFETY: the handle is the initial thread of the suspended child.
    if unsafe { ResumeThread(thread.0) } == u32::MAX {
        return Err(last_os_error("ResumeThread"));
    }

    // SAFETY: the child process handle remains owned until this function exits.
    let waited = unsafe { WaitForSingleObject(process.0, INFINITE) };
    if waited != WAIT_OBJECT_0 {
        return Err(last_os_error("WaitForSingleObject"));
    }
    let mut exit_code = 1;
    // SAFETY: the child process has completed and the output pointer is valid.
    if unsafe { GetExitCodeProcess(process.0, &mut exit_code) } == 0 {
        return Err(last_os_error("GetExitCodeProcess"));
    }
    Ok(exit_code)
}

fn open_current_process_token() -> io::Result<OwnedHandle> {
    let mut token = null_mut();
    // SAFETY: GetCurrentProcess returns a valid pseudo-handle; token points to
    // writable storage for the resulting handle.
    if unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_QUERY,
            &mut token,
        )
    } == 0
    {
        return Err(last_os_error("OpenProcessToken"));
    }
    OwnedHandle::new(token, "process token")
}

fn restricted_code_sid() -> io::Result<OwnedSid> {
    let sid_text = wide_nul(OsStr::new("S-1-5-12"));
    let mut sid = null_mut();
    // SAFETY: sid_text is NUL-terminated and sid is writable output storage.
    if unsafe { ConvertStringSidToSidW(sid_text.as_ptr(), &mut sid) } == 0 {
        return Err(last_os_error("ConvertStringSidToSidW"));
    }
    if sid.is_null() {
        return Err(io::Error::other(
            "ConvertStringSidToSidW returned a null SID",
        ));
    }
    Ok(OwnedSid(sid))
}

fn create_write_restricted_token(
    process_token: HANDLE,
    restricted_sid: &SID_AND_ATTRIBUTES,
) -> io::Result<OwnedHandle> {
    let mut token = null_mut();
    // WRITE_RESTRICTED makes the restricted-code SID participate in write
    // access checks while leaving normal read and network access available.
    // SAFETY: the source token and SID are valid, the SID attributes are zero,
    // and token points to writable output storage.
    if unsafe {
        CreateRestrictedToken(
            process_token,
            DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED,
            0,
            null(),
            0,
            null(),
            1,
            restricted_sid,
            &mut token,
        )
    } == 0
    {
        return Err(last_os_error("CreateRestrictedToken"));
    }
    OwnedHandle::new(token, "restricted worker token")
}

fn create_kill_on_close_job() -> io::Result<OwnedHandle> {
    // SAFETY: null security attributes create a non-inheritable unnamed job.
    let raw_job = unsafe { CreateJobObjectW(null(), null()) };
    let job = OwnedHandle::new(raw_job, "worker job")?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: limits has the exact structure size required for this information
    // class and remains alive throughout the call.
    if unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    } == 0
    {
        return Err(last_os_error("SetInformationJobObject"));
    }
    let ui_limits = JOBOBJECT_BASIC_UI_RESTRICTIONS {
        UIRestrictionsClass: JOB_OBJECT_UILIMIT_HANDLES
            | JOB_OBJECT_UILIMIT_READCLIPBOARD
            | JOB_OBJECT_UILIMIT_WRITECLIPBOARD
            | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
            | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
            | JOB_OBJECT_UILIMIT_GLOBALATOMS
            | JOB_OBJECT_UILIMIT_DESKTOP
            | JOB_OBJECT_UILIMIT_EXITWINDOWS,
    };
    // Also prevent the restricted worker from using desktop handles or
    // interacting with the browser's windows and clipboard.
    // SAFETY: ui_limits is the exact structure for this job information class.
    if unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectBasicUIRestrictions,
            &ui_limits as *const _ as *const _,
            size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
        )
    } == 0
    {
        return Err(last_os_error("SetInformationJobObject(UI restrictions)"));
    }
    Ok(job)
}

fn std_handle(which: u32) -> io::Result<HANDLE> {
    // SAFETY: GetStdHandle has no pointer arguments and returns a borrowed
    // process standard handle.
    let handle = unsafe { GetStdHandle(which) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        Err(last_os_error("GetStdHandle"))
    } else {
        Ok(handle)
    }
}

fn worker_command_line(executable: &OsStr) -> Vec<u16> {
    let mut command_line = quote_windows_argument(executable);
    command_line.pop(); // Remove the first argument's terminating NUL.
    command_line.push(b' ' as u16);
    command_line.extend(quote_windows_argument(OsStr::new(super::WORKER_ARG)));
    command_line
}

fn quote_windows_argument(argument: &OsStr) -> Vec<u16> {
    let mut quoted = vec![b'"' as u16];
    let mut backslashes = 0usize;
    for unit in argument.encode_wide() {
        if unit == b'\\' as u16 {
            backslashes += 1;
        } else if unit == b'"' as u16 {
            quoted.extend(std::iter::repeat_n(b'\\' as u16, backslashes * 2 + 1));
            quoted.push(unit);
            backslashes = 0;
        } else {
            quoted.extend(std::iter::repeat_n(b'\\' as u16, backslashes));
            quoted.push(unit);
            backslashes = 0;
        }
    }
    quoted.extend(std::iter::repeat_n(b'\\' as u16, backslashes * 2));
    quoted.push(b'"' as u16);
    quoted.push(0);
    quoted
}

fn wide_nul(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn last_os_error(action: &str) -> io::Error {
    let code = unsafe { GetLastError() };
    io::Error::other(format!("{action} failed with Windows error {code}"))
}

struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn new(handle: HANDLE, name: &str) -> io::Result<Self> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            Err(last_os_error(&format!("creating {name}")))
        } else {
            Ok(Self(handle))
        }
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper takes ownership of a valid Win32 handle exactly
        // once and closes it when leaving scope.
        unsafe { CloseHandle(self.0) };
    }
}

struct OwnedSid(*mut core::ffi::c_void);

impl Drop for OwnedSid {
    fn drop(&mut self) {
        // SAFETY: ConvertStringSidToSidW allocated this SID with LocalAlloc.
        unsafe { LocalFree(self.0) };
    }
}
