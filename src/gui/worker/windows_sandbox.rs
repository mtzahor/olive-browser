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
    Foundation::{
        CloseHandle, GetHandleInformation, GetLastError, HANDLE, HANDLE_FLAG_INHERIT,
        INVALID_HANDLE_VALUE, LocalFree, SetHandleInformation, WAIT_OBJECT_0,
    },
    Security::Authorization::ConvertStringSidToSidW,
    Security::{
        ACL_REVISION, AddAccessAllowedAce, CreateRestrictedToken, DISABLE_MAX_PRIVILEGE,
        GetTokenInformation, InitializeAcl, InitializeSecurityDescriptor, LUA_TOKEN,
        SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR, SID_AND_ATTRIBUTES, SetSecurityDescriptorDacl,
        TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_QUERY, TOKEN_USER, TokenUser,
        WRITE_RESTRICTED,
    },
    System::{
        Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE},
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
        StationsAndDesktops::{
            CloseDesktop, CloseWindowStation, CreateDesktopW, CreateWindowStationW,
            GetProcessWindowStation, HDESK, HWINSTA, SetProcessWindowStation,
        },
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateProcessAsUserW, GetCurrentProcess,
            GetCurrentProcessId, GetExitCodeProcess, INFINITE, OpenProcessToken,
            PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOW,
            TerminateProcess, WaitForSingleObject,
        },
    },
    UI::WindowsAndMessaging::WINSTA_ALL_ACCESS,
};

const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
const DESKTOP_ALL_ACCESS: u32 = 0x000F01FF;

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
    let worker_desktop = create_worker_desktop(process_token.0, restricted_sid.0)?;
    startup.lpDesktop = worker_desktop.path.as_ptr();
    // std::process may provide non-inheritable duplicates for this broker's
    // pipes. The nested worker needs these three protocol handles explicitly
    // marked inheritable while CreateProcessAsUserW starts it.
    let inherited_handles =
        InheritableHandles::new([startup.hStdInput, startup.hStdOutput, startup.hStdError])?;

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
    let create_error = (created == 0).then(|| last_os_error("CreateProcessAsUserW"));
    drop(inherited_handles);
    if let Some(error) = create_error {
        return Err(error);
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

fn create_worker_desktop(
    process_token: HANDLE,
    restricted_sid: *mut core::ffi::c_void,
) -> io::Result<WorkerDesktop> {
    let user_sid = token_user_sid(process_token)?;
    let station_security =
        ObjectSecurity::new([user_sid.sid, restricted_sid], WINSTA_ALL_ACCESS as u32)?;
    let desktop_security = ObjectSecurity::new([user_sid.sid, restricted_sid], DESKTOP_ALL_ACCESS)?;

    let station_name = wide_nul(OsStr::new(&format!("OliveWorker-{}", unsafe {
        GetCurrentProcessId()
    })));
    let desktop_name = wide_nul(OsStr::new("Worker"));
    let desktop_path = wide_nul(OsStr::new(&format!(
        "{}\\Worker",
        String::from_utf16_lossy(&station_name[..station_name.len() - 1])
    )));

    // The restricted token needs its own non-interactive window station and
    // desktop. The DACL grants access only to the current user and the
    // Restricted Code SID used by the write-restricted token.
    // SAFETY: the name is NUL-terminated and the security descriptor and ACL
    // remain alive for the duration of the call.
    let raw_station = unsafe {
        CreateWindowStationW(
            station_name.as_ptr(),
            0,
            WINSTA_ALL_ACCESS as u32,
            &station_security.attributes,
        )
    };
    let station = OwnedWindowStation::new(raw_station)?;
    // SAFETY: this broker has no UI threads or windows; the original station
    // is restored by WorkerDesktop::drop after the restricted child exits.
    let original_station = unsafe { GetProcessWindowStation() };
    if original_station.is_null() {
        return Err(last_os_error("GetProcessWindowStation"));
    }
    // SAFETY: station is a valid window-station handle created above.
    if unsafe { SetProcessWindowStation(station.0) } == 0 {
        return Err(last_os_error("SetProcessWindowStation"));
    }
    // SAFETY: desktop_name is NUL-terminated and its DACL remains alive for
    // the duration of the call. A null device and DEVMODE select defaults.
    let raw_desktop = unsafe {
        CreateDesktopW(
            desktop_name.as_ptr(),
            null(),
            null(),
            0,
            DESKTOP_ALL_ACCESS,
            &desktop_security.attributes,
        )
    };
    if raw_desktop.is_null() {
        let error = last_os_error("CreateDesktopW");
        // SAFETY: original_station is the borrowed station handle captured
        // before switching this process to the private station.
        unsafe { SetProcessWindowStation(original_station) };
        return Err(error);
    }
    Ok(WorkerDesktop {
        _station: station,
        _desktop: OwnedDesktop(raw_desktop),
        original_station,
        path: desktop_path,
    })
}

struct TokenUserSid {
    _storage: Vec<usize>,
    sid: *mut core::ffi::c_void,
}

fn token_user_sid(token: HANDLE) -> io::Result<TokenUserSid> {
    let mut required = 0u32;
    // SAFETY: this size query uses a null output pointer and requests the
    // required buffer length for TokenUser.
    unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut required) };
    if required == 0 {
        return Err(last_os_error("GetTokenInformation(TokenUser size)"));
    }
    let words = (required as usize).div_ceil(size_of::<usize>());
    let mut storage = vec![0usize; words];
    // SAFETY: storage is pointer-aligned and large enough for the size queried
    // above; the API initializes a TOKEN_USER structure in this buffer.
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            storage.as_mut_ptr().cast(),
            (storage.len() * size_of::<usize>()) as u32,
            &mut required,
        )
    } == 0
    {
        return Err(last_os_error("GetTokenInformation(TokenUser)"));
    }
    // SAFETY: the successful TokenUser query wrote a valid TOKEN_USER at the
    // start of the aligned buffer, which remains owned by the returned value.
    let user = unsafe { &*(storage.as_ptr().cast::<TOKEN_USER>()) };
    if user.User.Sid.is_null() {
        return Err(io::Error::other("TokenUser returned a null SID"));
    }
    Ok(TokenUserSid {
        _storage: storage,
        sid: user.User.Sid,
    })
}

struct ObjectSecurity {
    _acl: Vec<u64>,
    _descriptor: Box<SECURITY_DESCRIPTOR>,
    attributes: SECURITY_ATTRIBUTES,
}

impl ObjectSecurity {
    fn new(sids: [*mut core::ffi::c_void; 2], access: u32) -> io::Result<Self> {
        // Two ACEs fit comfortably in this aligned ACL buffer.
        let mut acl = vec![0u64; 128];
        let acl_ptr = acl.as_mut_ptr().cast();
        // SAFETY: acl_ptr addresses a writable, correctly aligned buffer of
        // 1024 bytes, enough for two access-allowed ACEs and their SIDs.
        if unsafe { InitializeAcl(acl_ptr, (acl.len() * size_of::<u64>()) as u32, ACL_REVISION) }
            == 0
        {
            return Err(last_os_error("InitializeAcl"));
        }
        for sid in sids {
            // SAFETY: each SID is valid for this call and acl_ptr has capacity
            // for both entries.
            if unsafe { AddAccessAllowedAce(acl_ptr, ACL_REVISION, access, sid) } == 0 {
                return Err(last_os_error("AddAccessAllowedAce"));
            }
        }
        let mut descriptor = Box::<SECURITY_DESCRIPTOR>::default();
        // SAFETY: descriptor and ACL are live, writable Win32 structures.
        if unsafe {
            InitializeSecurityDescriptor(
                (&mut *descriptor as *mut SECURITY_DESCRIPTOR).cast(),
                SECURITY_DESCRIPTOR_REVISION,
            )
        } == 0
        {
            return Err(last_os_error("InitializeSecurityDescriptor"));
        }
        // SAFETY: the descriptor points to the live ACL owned by this value.
        if unsafe {
            SetSecurityDescriptorDacl(
                (&mut *descriptor as *mut SECURITY_DESCRIPTOR).cast(),
                1,
                acl_ptr,
                0,
            )
        } == 0
        {
            return Err(last_os_error("SetSecurityDescriptorDacl"));
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: (&mut *descriptor as *mut SECURITY_DESCRIPTOR).cast(),
            bInheritHandle: 0,
        };
        Ok(Self {
            _acl: acl,
            _descriptor: descriptor,
            attributes,
        })
    }
}

struct WorkerDesktop {
    _station: OwnedWindowStation,
    _desktop: OwnedDesktop,
    original_station: HWINSTA,
    path: Vec<u16>,
}

impl Drop for WorkerDesktop {
    fn drop(&mut self) {
        // SAFETY: original_station is the borrowed station handle captured
        // before switching this launcher to its private window station.
        unsafe { SetProcessWindowStation(self.original_station) };
    }
}

struct OwnedWindowStation(HWINSTA);

impl OwnedWindowStation {
    fn new(handle: HWINSTA) -> io::Result<Self> {
        if handle.is_null() {
            Err(last_os_error("CreateWindowStationW"))
        } else {
            Ok(Self(handle))
        }
    }
}

impl Drop for OwnedWindowStation {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the handle returned by CreateWindowStationW.
        unsafe { CloseWindowStation(self.0) };
    }
}

struct OwnedDesktop(HDESK);

impl Drop for OwnedDesktop {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the handle returned by CreateDesktopW.
        unsafe { CloseDesktop(self.0) };
    }
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

struct InheritableHandles(Vec<(HANDLE, u32)>);

impl InheritableHandles {
    fn new(handles: [HANDLE; 3]) -> io::Result<Self> {
        let mut original_flags = Vec::with_capacity(handles.len());
        for handle in handles {
            let mut flags = 0;
            // SAFETY: handle is a valid borrowed standard handle and flags is
            // writable storage for the handle flags.
            if unsafe { GetHandleInformation(handle, &mut flags) } == 0 {
                restore_inherit_flags(&original_flags);
                return Err(last_os_error("GetHandleInformation"));
            }
            original_flags.push((handle, flags));
            // SAFETY: only the inherit bit is changed; the borrowed handle
            // remains owned by the Rust process launcher.
            if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) }
                == 0
            {
                let error = last_os_error("SetHandleInformation");
                restore_inherit_flags(&original_flags);
                return Err(error);
            }
        }
        Ok(Self(original_flags))
    }
}

impl Drop for InheritableHandles {
    fn drop(&mut self) {
        restore_inherit_flags(&self.0);
    }
}

fn restore_inherit_flags(handles: &[(HANDLE, u32)]) {
    for &(handle, flags) in handles.iter().rev() {
        // SAFETY: these are the borrowed standard handles recorded by
        // InheritableHandles::new; restore only their original inherit bit.
        unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, flags & HANDLE_FLAG_INHERIT) };
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
